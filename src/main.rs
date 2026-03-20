mod tmux;

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    layout::Rect,
    DefaultTerminal, Frame,
};

use ansi_to_tui::IntoText as _;
use tmux::{Session, Status};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

const PREVIEW_SCROLL_STEP: u16 = 10;

enum Mode {
    Normal,
    Filter,
    Help,
}

struct App {
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    list_state: ListState,
    last_refresh: Instant,
    preview: String,
    preview_scroll: u16,
    preview_auto_scroll: bool,
    mode: Mode,
    filter_query: String,
    changed: HashMap<String, bool>,
    prev_statuses: HashMap<String, Status>,
    show_git: bool,
    auto_sort: bool,
}

impl App {
    fn new() -> Result<Self> {
        let sessions = tmux::detect_sessions().unwrap_or_default();
        let filtered: Vec<usize> = (0..sessions.len()).collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }
        Ok(Self {
            sessions,
            filtered,
            list_state,
            last_refresh: Instant::now(),
            preview: String::new(),
            preview_scroll: 0,
            preview_auto_scroll: true,
            mode: Mode::Normal,
            filter_query: String::new(),
            changed: HashMap::new(),
            prev_statuses: HashMap::new(),
            show_git: false,
            auto_sort: false,
        })
    }

    fn refresh(&mut self) {
        if self.last_refresh.elapsed() < REFRESH_INTERVAL {
            return;
        }
        let selected_target = self.selected_session().map(|s| s.target.clone());
        self.sessions = tmux::detect_sessions().unwrap_or_default();
        if self.auto_sort {
            self.sessions.sort_by(|a, b| a.status.cmp(&b.status));
        }

        // Detect status changes and notify on new approval requests
        for session in &self.sessions {
            if let Some(prev) = self.prev_statuses.get(&session.target) {
                if *prev != session.status {
                    self.changed.insert(session.target.clone(), true);
                    if session.status == Status::WaitingForApproval {
                        let _ = tmux::notify(&format!(
                            "{} needs approval", session.label()
                        ));
                    }
                }
            }
        }
        self.prev_statuses = self.sessions.iter()
            .map(|s| (s.target.clone(), s.status.clone()))
            .collect();

        self.apply_filter();

        // Preserve selection by target, or clamp to bounds
        let new_index = selected_target
            .and_then(|t| self.filtered.iter().position(|&i| self.sessions[i].target == t))
            .or(if self.filtered.is_empty() { None } else { Some(0) });
        self.list_state.select(new_index);
        self.refresh_preview();
        self.last_refresh = Instant::now();
    }

    fn apply_filter(&mut self) {
        let query = self.filter_query.to_lowercase();
        self.filtered = self.sessions.iter().enumerate()
            .filter(|(_, s)| {
                query.is_empty()
                    || s.label().to_lowercase().contains(&query)
                    || s.cwd.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn refresh_preview(&mut self) {
        if let Some(target) = self.selected_session().map(|s| s.target.clone()) {
            self.changed.remove(&target);
        }
        self.preview = self
            .selected_session()
            .and_then(|s| tmux::capture_pane(&s.target).ok())
            .unwrap_or_default();
        self.preview_auto_scroll = true;
    }

    fn scroll_preview_down(&mut self) {
        self.preview_auto_scroll = false;
        let line_count = self.preview.lines().count() as u16;
        self.preview_scroll = self.preview_scroll.saturating_add(PREVIEW_SCROLL_STEP).min(line_count.saturating_sub(1));
    }

    fn scroll_preview_up(&mut self) {
        self.preview_auto_scroll = false;
        self.preview_scroll = self.preview_scroll.saturating_sub(PREVIEW_SCROLL_STEP);
    }

    fn selected_session(&self) -> Option<&Session> {
        self.list_state.selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&i| &self.sessions[i])
    }

    fn next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % self.filtered.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
        self.refresh_preview();
    }

    fn previous(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) | None => self.filtered.len() - 1,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
        self.refresh_preview();
    }

    fn jump_to_selected(&self) {
        if let Some(session) = self.selected_session() {
            let _ = tmux::switch_to_pane(&session.target);
        }
    }

    fn select_option(&mut self, option: u8) {
        if let Some(session) = self.selected_session() {
            if session.status == Status::WaitingForApproval {
                let _ = tmux::select_option(&session.target, option);
                self.last_refresh = Instant::now() - REFRESH_INTERVAL;
            }
        }
    }
}

fn main() -> Result<()> {
    color_eyre::install()?;
    io::stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let terminal = ratatui::init();
    let result = run(terminal);

    io::stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;

    result
}

fn run(mut terminal: DefaultTerminal) -> Result<()> {
    let mut app = App::new()?;

    loop {
        app.refresh();
        terminal.draw(|frame| ui(frame, &mut app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                match app.mode {
                    Mode::Normal => match (key.code, shift) {
                        (KeyCode::Char('q'), _) => break,
                        (KeyCode::Char('J'), true) => app.scroll_preview_down(),
                        (KeyCode::Char('K'), true) => app.scroll_preview_up(),
                        (KeyCode::Char('j'), false) | (KeyCode::Down, false) => app.next(),
                        (KeyCode::Char('k'), false) | (KeyCode::Up, false) => app.previous(),
                        (KeyCode::Enter, _) | (KeyCode::Char('l'), false) => app.jump_to_selected(),
                        (KeyCode::Char('a'), false) | (KeyCode::Char('1'), false) => app.select_option(1),
                        (KeyCode::Char('2'), false) => app.select_option(2),
                        (KeyCode::Char('3'), false) => app.select_option(3),
                        (KeyCode::Char('w'), false) => app.show_git = !app.show_git,
                        (KeyCode::Char('s'), false) => {
                            app.auto_sort = !app.auto_sort;
                            app.last_refresh = Instant::now() - REFRESH_INTERVAL;
                        }
                        (KeyCode::Char('/'), false) => {
                            app.mode = Mode::Filter;
                            app.filter_query.clear();
                        }
                        (KeyCode::Char('?'), _) => {
                            app.mode = Mode::Help;
                        }
                        _ => {}
                    },
                    Mode::Help => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                            app.mode = Mode::Normal;
                        }
                        _ => {}
                    },
                    Mode::Filter => match key.code {
                        KeyCode::Esc => {
                            app.mode = Mode::Normal;
                            app.filter_query.clear();
                            app.apply_filter();
                            if !app.filtered.is_empty() {
                                app.list_state.select(Some(0));
                            }
                            app.refresh_preview();
                        }
                        KeyCode::Enter => {
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Backspace => {
                            app.filter_query.pop();
                            app.apply_filter();
                            if !app.filtered.is_empty() {
                                app.list_state.select(Some(0));
                            }
                            app.refresh_preview();
                        }
                        KeyCode::Char(c) => {
                            app.filter_query.push(c);
                            app.apply_filter();
                            if !app.filtered.is_empty() {
                                app.list_state.select(Some(0));
                            }
                            app.refresh_preview();
                        }
                        _ => {}
                    },
                }
            }
        }
    }
    Ok(())
}

fn ui(frame: &mut Frame, app: &mut App) {
    let [header, body, help_bar] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
            .areas(frame.area());

    let help_text = match app.mode {
        Mode::Filter => "Type to filter · Enter confirm · Esc clear",
        Mode::Help => "Press ? or Esc to close",
        Mode::Normal => "j/k navigate · l/Enter jump · a approve · / filter · J/K scroll · w git · ? help · q quit",
    };
    frame.render_widget(
        Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray)),
        help_bar,
    );

    // Session summary counts
    let approval_count = app.sessions.iter().filter(|s| s.status == Status::WaitingForApproval).count();
    let working_count = app.sessions.iter().filter(|s| matches!(s.status, Status::Working(_))).count();
    let idle_count = app.sessions.iter().filter(|s| s.status == Status::Idle).count();

    let mut title_spans = vec![
        Span::styled(
            format!(" {} sessions", app.sessions.len()),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    ];
    if !app.sessions.is_empty() {
        title_spans.push(Span::styled("  ", Style::default()));
        if approval_count > 0 {
            title_spans.push(Span::styled(format!("{approval_count} ⚠"), Style::default().fg(Color::Yellow)));
            title_spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
        if working_count > 0 {
            title_spans.push(Span::styled(format!("{working_count} ◉"), Style::default().fg(Color::Cyan)));
            title_spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
        title_spans.push(Span::styled(format!("{idle_count} ●"), Style::default().fg(Color::DarkGray)));
    }

    // Active toggles
    let mut toggles = Vec::new();
    if app.auto_sort { toggles.push("sort"); }
    if app.show_git { toggles.push("git"); }
    if !toggles.is_empty() {
        title_spans.push(Span::styled("  ", Style::default()));
        for (i, t) in toggles.iter().enumerate() {
            if i > 0 { title_spans.push(Span::styled(" ", Style::default())); }
            title_spans.push(Span::styled(
                format!("[{t}]"),
                Style::default().fg(Color::Magenta),
            ));
        }
    }

    let title = Paragraph::new(Line::from(title_spans))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, header);

    if app.sessions.is_empty() {
        let empty = Paragraph::new("No Claude Code sessions detected.")
            .block(Block::default().title(" Sessions ").borders(Borders::ALL));
        frame.render_widget(empty, body);
        return;
    }

    // Show filter bar when in filter mode or when a filter is active
    let body = if matches!(app.mode, Mode::Filter) || !app.filter_query.is_empty() {
        let [body, filter_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);
        let filter_text = format!("/{}", app.filter_query);
        let filter_bar = Paragraph::new(filter_text)
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(filter_bar, filter_area);
        body
    } else {
        body
    };

    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(body);

    let mut items: Vec<ListItem> = Vec::new();
    let mut last_group = String::new();
    for &i in &app.filtered {
        let s = &app.sessions[i];
        let (indicator, indicator_color) = match &s.status {
            Status::Idle => ("●", Color::DarkGray),
            Status::Working(_) => ("◉", Color::Cyan),
            Status::WaitingForApproval => ("⚠", Color::Yellow),
        };
        let changed = app.changed.contains_key(&s.target);
        let group = s.label();
        let show_group = group != last_group;
        if show_group {
            last_group = group.to_string();
        }
        let mut lines = Vec::new();
        if show_group {
            lines.push(Line::from(Span::styled(
                format!("┌ {group}"),
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            )));
        }
        let mut spans = vec![
            Span::styled(
                format!("{indicator} "),
                Style::default().fg(indicator_color),
            ),
            Span::styled(
                s.short_cwd().to_string(),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("  {}", s.status),
                Style::default().fg(indicator_color),
            ),
        ];
        if changed {
            spans.push(Span::styled(
                " *",
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(spans));
        if app.show_git {
            if let Some(git) = &s.git {
                let tag = if git.is_worktree { " [worktree]" } else { "" };
                lines.push(Line::from(Span::styled(
                    format!("  {}{tag}", git.branch),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        items.push(ListItem::new(lines));
    }

    let count = items.len();
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Sessions ({count}) "))
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, list_area, &mut app.list_state);

    let preview_title = app
        .selected_session()
        .map(|s| format!(" {} ", s.target))
        .unwrap_or_else(|| " Preview ".to_string());

    let preview_text = app.preview.into_text().unwrap_or_default();

    if app.preview_auto_scroll {
        let line_count = preview_text.lines.len() as u16;
        // preview_area height minus 2 for borders
        let visible = preview_area.height.saturating_sub(2);
        app.preview_scroll = line_count.saturating_sub(visible);
    }

    let preview = Paragraph::new(preview_text)
        .block(
            Block::default()
                .title(preview_title)
                .borders(Borders::ALL),
        )
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview, preview_area);

    // Help overlay
    if matches!(app.mode, Mode::Help) {
        let area = centered_rect(60, 70, frame.area());
        let help_content = vec![
            Line::from(Span::styled(" Keybindings ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![Span::styled("  j / ↓       ", Style::default().fg(Color::Green)), Span::raw("Move down in the session list")]),
            Line::from(vec![Span::styled("  k / ↑       ", Style::default().fg(Color::Green)), Span::raw("Move up in the session list")]),
            Line::from(vec![Span::styled("  l / Enter   ", Style::default().fg(Color::Green)), Span::raw("Jump to the selected session")]),
            Line::from(vec![Span::styled("  a / 1       ", Style::default().fg(Color::Green)), Span::raw("Select option 1 (Yes)")]),
            Line::from(vec![Span::styled("  2           ", Style::default().fg(Color::Green)), Span::raw("Select option 2 (Yes, don't ask again)")]),
            Line::from(vec![Span::styled("  3           ", Style::default().fg(Color::Green)), Span::raw("Select option 3 (No)")]),
            Line::from(vec![Span::styled("  J (shift)   ", Style::default().fg(Color::Green)), Span::raw("Scroll preview down")]),
            Line::from(vec![Span::styled("  K (shift)   ", Style::default().fg(Color::Green)), Span::raw("Scroll preview up")]),
            Line::from(vec![Span::styled("  /           ", Style::default().fg(Color::Green)), Span::raw("Filter sessions")]),
            Line::from(vec![Span::styled("  s           ", Style::default().fg(Color::Green)), Span::raw("Toggle auto-sort by priority")]),
            Line::from(vec![Span::styled("  w           ", Style::default().fg(Color::Green)), Span::raw("Toggle git branch / worktree info")]),
            Line::from(vec![Span::styled("  ?           ", Style::default().fg(Color::Green)), Span::raw("Toggle this help")]),
            Line::from(vec![Span::styled("  q           ", Style::default().fg(Color::Green)), Span::raw("Quit")]),
            Line::from(""),
            Line::from(Span::styled(" Status indicators ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![Span::styled("  ●  ", Style::default().fg(Color::DarkGray)), Span::raw("Idle — waiting for your input")]),
            Line::from(vec![Span::styled("  ◉  ", Style::default().fg(Color::Cyan)), Span::raw("Working — actively processing")]),
            Line::from(vec![Span::styled("  ⚠  ", Style::default().fg(Color::Yellow)), Span::raw("Needs approval — permission prompt")]),
            Line::from(vec![Span::styled("  *  ", Style::default().fg(Color::Magenta)), Span::raw("Status changed since last viewed")]),
        ];
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(help_content)
                .block(Block::default().title(" Help ").borders(Borders::ALL))
                .style(Style::default().bg(Color::Black)),
            area,
        );
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [_, vert, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ]).areas(area);
    let [_, horiz, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ]).areas(vert);
    horiz
}
