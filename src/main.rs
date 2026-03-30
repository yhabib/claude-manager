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
use tmux::{Session, Status, TokenUsage};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const COST_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

const PREVIEW_SCROLL_STEP: u16 = 10;

enum Mode {
    Normal,
    Filter,
    Prompt,
    Help,
    AddSession,
}

struct App {
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    list_state: ListState,
    last_refresh: Instant,
    preview: String,
    preview_scroll: u16,
    preview_pinned: bool,
    mode: Mode,
    filter_query: String,
    prompt_input: String,
    changed: HashMap<String, bool>,
    prev_statuses: HashMap<String, Status>,
    show_git: bool,
    auto_sort: bool,
    daily_cost: TokenUsage,
    monthly_cost: TokenUsage,
    last_cost_refresh: Instant,
    add_session_items: Vec<(String, String)>,
    add_session_state: ListState,
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
            preview_scroll: u16::MAX,
            preview_pinned: false,
            mode: Mode::Normal,
            filter_query: String::new(),
            prompt_input: String::new(),
            changed: HashMap::new(),
            prev_statuses: HashMap::new(),
            show_git: false,
            auto_sort: false,
            daily_cost: TokenUsage::default(),
            monthly_cost: TokenUsage::default(),
            last_cost_refresh: Instant::now() - COST_REFRESH_INTERVAL,
            add_session_items: vec![],
            add_session_state: ListState::default(),
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

        // Refresh daily/monthly costs every 60s
        if self.last_cost_refresh.elapsed() >= COST_REFRESH_INTERVAL {
            let (daily, monthly) = tmux::read_period_usage();
            self.daily_cost = daily;
            self.monthly_cost = monthly;
            self.last_cost_refresh = Instant::now();
        }

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
    }

    fn switch_preview(&mut self) {
        self.refresh_preview();
        self.preview_scroll = u16::MAX;
        self.preview_pinned = false;
    }

    fn scroll_preview_down(&mut self) {
        self.preview_pinned = true;
        let line_count = self.preview.lines().count() as u16;
        self.preview_scroll = self.preview_scroll.saturating_add(PREVIEW_SCROLL_STEP).min(line_count.saturating_sub(1));
    }

    fn scroll_preview_up(&mut self) {
        self.preview_pinned = true;
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
        self.switch_preview();
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
        self.switch_preview();
    }

    fn jump_to_selected(&self) {
        if let Some(session) = self.selected_session() {
            let _ = tmux::switch_to_pane(&session.target);
        }
    }

    fn open_lazygit(&self) {
        if let Some(session) = self.selected_session() {
            if !session.cwd.is_empty() {
                let _ = tmux::open_lazygit(&session.cwd);
            }
        }
    }

    fn send_prompt(&mut self, text: &str) {
        if let Some(session) = self.selected_session() {
            if session.status == Status::Idle {
                let _ = tmux::send_keys(&session.target, &[text, "Enter"]);
                self.last_refresh = Instant::now() - REFRESH_INTERVAL;
            }
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

    fn open_add_session(&mut self) {
        let existing: std::collections::HashSet<String> =
            self.sessions.iter().map(|s| s.target.clone()).collect();
        self.add_session_items = tmux::list_all_panes()
            .unwrap_or_default()
            .into_iter()
            .filter(|(t, _)| !existing.contains(t))
            .collect();
        self.add_session_state = ListState::default();
        if !self.add_session_items.is_empty() {
            self.add_session_state.select(Some(0));
        }
        self.mode = Mode::AddSession;
    }

    fn confirm_add_session(&mut self) {
        if let Some(i) = self.add_session_state.selected() {
            if let Some((target, _)) = self.add_session_items.get(i) {
                let mut pinned = tmux::load_pinned();
                if !pinned.contains(target) {
                    pinned.push(target.clone());
                    tmux::save_pinned(&pinned);
                }
                self.last_refresh = Instant::now() - REFRESH_INTERVAL;
            }
        }
        self.mode = Mode::Normal;
    }

    fn unpin_selected(&mut self) {
        if let Some(session) = self.selected_session() {
            if session.pinned {
                let target = session.target.clone();
                let mut pinned = tmux::load_pinned();
                pinned.retain(|t| t != &target);
                tmux::save_pinned(&pinned);
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
                        (KeyCode::Char('1'), false) => app.select_option(1),
                        (KeyCode::Char('2'), false) => app.select_option(2),
                        (KeyCode::Char('3'), false) => app.select_option(3),
                        (KeyCode::Char('g'), false) => app.open_lazygit(),
                        (KeyCode::Char('w'), false) => app.show_git = !app.show_git,
                        (KeyCode::Char('s'), false) => {
                            app.auto_sort = !app.auto_sort;
                            app.last_refresh = Instant::now() - REFRESH_INTERVAL;
                        }
                        (KeyCode::Char('p'), false) => {
                            app.mode = Mode::Prompt;
                            app.prompt_input.clear();
                        }
                        (KeyCode::Char('/'), false) => {
                            app.mode = Mode::Filter;
                            app.filter_query.clear();
                        }
                        (KeyCode::Char('?'), _) => {
                            app.mode = Mode::Help;
                        }
                        (KeyCode::Char('a'), false) => app.open_add_session(),
                        (KeyCode::Char('d'), false) => app.unpin_selected(),
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
                            app.switch_preview();
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
                            app.switch_preview();
                        }
                        KeyCode::Char(c) => {
                            app.filter_query.push(c);
                            app.apply_filter();
                            if !app.filtered.is_empty() {
                                app.list_state.select(Some(0));
                            }
                            app.switch_preview();
                        }
                        _ => {}
                    },
                    Mode::Prompt => match key.code {
                        KeyCode::Esc => {
                            app.mode = Mode::Normal;
                            app.prompt_input.clear();
                        }
                        KeyCode::Enter => {
                            let input = app.prompt_input.clone();
                            if !input.is_empty() {
                                app.send_prompt(&input);
                            }
                            app.prompt_input.clear();
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Backspace => {
                            app.prompt_input.pop();
                        }
                        KeyCode::Char(c) => {
                            app.prompt_input.push(c);
                        }
                        _ => {}
                    },
                    Mode::AddSession => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Enter => app.confirm_add_session(),
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !app.add_session_items.is_empty() {
                                let i = match app.add_session_state.selected() {
                                    Some(i) => (i + 1) % app.add_session_items.len(),
                                    None => 0,
                                };
                                app.add_session_state.select(Some(i));
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if !app.add_session_items.is_empty() {
                                let i = match app.add_session_state.selected() {
                                    Some(0) | None => app.add_session_items.len() - 1,
                                    Some(i) => i - 1,
                                };
                                app.add_session_state.select(Some(i));
                            }
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
        Mode::Prompt => "Type a prompt · Enter send · Esc cancel",
        Mode::Help => "Press ? or Esc to close",
        Mode::AddSession => "j/k navigate · Enter pin · Esc cancel",
        Mode::Normal => "j/k navigate · J/K scroll · l jump · 1/2/3 approve · p prompt · g lazygit · / filter · a add · d unpin · s sort · w git · ? help · q quit",
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

    // Cost breakdown: session | today | month
    let session_cost: f64 = app.sessions.iter().map(|s| s.tokens.estimated_cost()).sum();
    let daily_cost = app.daily_cost.estimated_cost();
    let monthly_cost = app.monthly_cost.estimated_cost();
    if session_cost > 0.0 || daily_cost > 0.0 {
        title_spans.push(Span::styled("  ", Style::default()));
        title_spans.push(Span::styled(
            format!("session: ${:.2}", session_cost),
            Style::default().fg(Color::DarkGray),
        ));
        title_spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        title_spans.push(Span::styled(
            format!("today: ${:.2}", daily_cost),
            Style::default().fg(Color::DarkGray),
        ));
        title_spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        title_spans.push(Span::styled(
            format!("month: ${:.2}", monthly_cost),
            Style::default().fg(Color::Green),
        ));
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
    } else if matches!(app.mode, Mode::Prompt) {
        let [body, prompt_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);
        let target_name = app.selected_session()
            .map(|s| s.short_cwd().to_string())
            .unwrap_or_default();
        let prompt_text = format!("prompt ({target_name})> {}", app.prompt_input);
        let prompt_bar = Paragraph::new(prompt_text)
            .style(Style::default().fg(Color::Green));
        frame.render_widget(prompt_bar, prompt_area);
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
        let (indicator, indicator_color) = if s.pinned {
            ("○", Color::Blue)
        } else {
            match &s.status {
                Status::Idle => ("●", Color::DarkGray),
                Status::Working(_) => ("◉", Color::Cyan),
                Status::WaitingForApproval => ("⚠", Color::Yellow),
            }
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

    let line_count = preview_text.lines.len() as u16;
    let visible = preview_area.height.saturating_sub(2);
    let max_scroll = line_count.saturating_sub(visible);
    if !app.preview_pinned {
        app.preview_scroll = max_scroll;
    } else {
        app.preview_scroll = app.preview_scroll.min(max_scroll);
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
            Line::from(vec![Span::styled("  J (shift)   ", Style::default().fg(Color::Green)), Span::raw("Scroll preview down")]),
            Line::from(vec![Span::styled("  K (shift)   ", Style::default().fg(Color::Green)), Span::raw("Scroll preview up")]),
            Line::from(vec![Span::styled("  l / Enter   ", Style::default().fg(Color::Green)), Span::raw("Jump to the selected session")]),
            Line::from(vec![Span::styled("  1           ", Style::default().fg(Color::Green)), Span::raw("Select option 1 (Yes)")]),
            Line::from(vec![Span::styled("  2           ", Style::default().fg(Color::Green)), Span::raw("Select option 2 (Yes, don't ask again)")]),
            Line::from(vec![Span::styled("  3           ", Style::default().fg(Color::Green)), Span::raw("Select option 3 (No)")]),
            Line::from(vec![Span::styled("  p           ", Style::default().fg(Color::Green)), Span::raw("Send a prompt to selected session")]),
            Line::from(vec![Span::styled("  g           ", Style::default().fg(Color::Green)), Span::raw("Open lazygit for selected session")]),
            Line::from(vec![Span::styled("  /           ", Style::default().fg(Color::Green)), Span::raw("Filter sessions")]),
            Line::from(vec![Span::styled("  a           ", Style::default().fg(Color::Green)), Span::raw("Add a tmux session (pin)")]),
            Line::from(vec![Span::styled("  d           ", Style::default().fg(Color::Green)), Span::raw("Remove selected pinned session")]),
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
            Line::from(vec![Span::styled("  ○  ", Style::default().fg(Color::Blue)), Span::raw("Pinned — manually added tmux session")]),
            Line::from(vec![Span::styled("  *  ", Style::default().fg(Color::Magenta)), Span::raw("Status changed since last viewed")]),
            Line::from(""),
            Line::from(Span::styled(" Cost estimate ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from("  Based on Claude Opus 4.6 pricing."),
            Line::from("  Actual costs may differ by model."),
        ];
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(help_content)
                .block(Block::default().title(" Help ").borders(Borders::ALL))
                .style(Style::default().bg(Color::Black)),
            area,
        );
    }

    // Add session overlay
    if matches!(app.mode, Mode::AddSession) {
        let area = centered_rect(60, 60, frame.area());
        let items: Vec<ListItem> = if app.add_session_items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "  No other tmux panes found",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            app.add_session_items
                .iter()
                .map(|(target, cwd)| {
                    let short_cwd = cwd.rsplit('/').next().unwrap_or(cwd.as_str());
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("  {target}"), Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("  {short_cwd}"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]))
                })
                .collect()
        };
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .title(" Pin session (Enter add · Esc cancel) ")
                        .borders(Borders::ALL),
                )
                .style(Style::default().bg(Color::Black))
                .highlight_style(Style::default().bg(Color::Blue).fg(Color::White)),
            area,
            &mut app.add_session_state,
        );
    }
}

/// Extract all text content from a ratatui Buffer as a single string.
#[cfg(test)]
fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut result = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &buf[(x, y)];
            result.push_str(cell.symbol());
        }
        result.push('\n');
    }
    result
}

#[cfg(test)]
fn test_app(sessions: Vec<Session>) -> App {
    let filtered: Vec<usize> = (0..sessions.len()).collect();
    let mut list_state = ListState::default();
    if !filtered.is_empty() {
        list_state.select(Some(0));
    }
    App {
        sessions,
        filtered,
        list_state,
        last_refresh: Instant::now(),
        preview: String::new(),
        preview_scroll: 0,
        preview_pinned: false,
        mode: Mode::Normal,
        filter_query: String::new(),
        prompt_input: String::new(),
        changed: HashMap::new(),
        prev_statuses: HashMap::new(),
        show_git: false,
        auto_sort: false,
        daily_cost: TokenUsage::default(),
        monthly_cost: TokenUsage::default(),
        last_cost_refresh: Instant::now(),
        add_session_items: vec![],
        add_session_state: ListState::default(),
    }
}

#[cfg(test)]
fn make_session(target: &str, status: Status, cwd: &str) -> Session {
    Session {
        target: target.into(),
        status,
        cwd: cwd.into(),
        git: None,
        tokens: TokenUsage::default(),
        pinned: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use tmux::GitInfo;

    fn render(app: &mut App) -> String {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| ui(frame, app)).unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    // --- Header ---

    #[test]
    fn header_shows_session_count() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/home/user/proj"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("1 sessions"));
    }

    #[test]
    fn header_shows_status_counts() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
            make_session("b:0.0", Status::Working("Thinking…".into()), "/b"),
            make_session("c:0.0", Status::WaitingForApproval, "/c"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("3 sessions"));
        assert!(output.contains("1 ⚠"));
        assert!(output.contains("1 ◉"));
        assert!(output.contains("1 ●"));
    }

    #[test]
    fn header_shows_cost_when_tokens_present() {
        let mut app = test_app(vec![
            Session {
                target: "a:0.0".into(),
                status: Status::Idle,
                cwd: "/a".into(),
                git: None,
                tokens: TokenUsage {
                    input: 1_000_000,
                    output: 100_000,
                    cache_read: 0,
                    cache_write: 0,
                },
                pinned: false,
            },
        ]);
        let output = render(&mut app);
        assert!(output.contains("session: $"));
    }

    #[test]
    fn header_no_cost_when_no_tokens() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        let output = render(&mut app);
        assert!(!output.contains("session: $"));
    }

    #[test]
    fn header_shows_toggle_indicators() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.show_git = true;
        app.auto_sort = true;
        let output = render(&mut app);
        assert!(output.contains("[git]"));
        assert!(output.contains("[sort]"));
    }

    // --- Empty state ---

    #[test]
    fn empty_state_shows_no_sessions_message() {
        let mut app = test_app(vec![]);
        let output = render(&mut app);
        assert!(output.contains("No Claude Code sessions detected"));
    }

    // --- Session list ---

    #[test]
    fn session_list_shows_cwd_and_status() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/home/user/my-project"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("my-project"));
        assert!(output.contains("idle"));
    }

    #[test]
    fn session_list_shows_working_status() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Working("Reasoning…".into()), "/home/user/proj"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("Reasoning…"));
    }

    #[test]
    fn session_list_shows_group_header() {
        let mut app = test_app(vec![
            make_session("my-session:0.0", Status::Idle, "/home/user/proj"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("┌ my-session"));
    }

    #[test]
    fn session_list_shows_git_info_when_toggled() {
        let mut app = test_app(vec![
            Session {
                target: "proj:0.0".into(),
                status: Status::Idle,
                cwd: "/home/user/proj".into(),
                git: Some(GitInfo { branch: "feat/cool".into(), is_worktree: true }),
                tokens: TokenUsage::default(),
                pinned: false,
            },
        ]);
        // git off by default
        let output = render(&mut app);
        assert!(!output.contains("feat/cool"));

        // toggle on
        app.show_git = true;
        let output = render(&mut app);
        assert!(output.contains("feat/cool"));
        assert!(output.contains("[worktree]"));
    }

    #[test]
    fn session_list_shows_changed_marker() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/home/user/proj"),
        ]);
        app.changed.insert("proj:0.0".into(), true);
        // Select a different session or none so changed isn't cleared
        app.list_state.select(None);
        let output = render(&mut app);
        assert!(output.contains("*"));
    }

    // --- Help bar ---

    #[test]
    fn help_bar_normal_mode() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        let output = render(&mut app);
        assert!(output.contains("j/k navigate"));
        assert!(output.contains("/ filter"));
    }

    #[test]
    fn help_bar_filter_mode() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.mode = Mode::Filter;
        let output = render(&mut app);
        assert!(output.contains("Type to filter"));
        assert!(output.contains("Esc clear"));
    }

    #[test]
    fn help_bar_prompt_mode() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.mode = Mode::Prompt;
        let output = render(&mut app);
        assert!(output.contains("Type a prompt"));
        assert!(output.contains("Esc cancel"));
    }

    #[test]
    fn help_bar_help_mode() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.mode = Mode::Help;
        let output = render(&mut app);
        assert!(output.contains("Esc to close"));
    }

    // --- Filter bar ---

    #[test]
    fn filter_bar_shows_query() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.mode = Mode::Filter;
        app.filter_query = "proj".into();
        let output = render(&mut app);
        assert!(output.contains("/proj"));
    }

    // --- Prompt bar ---

    #[test]
    fn prompt_bar_shows_input() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/home/user/my-app"),
        ]);
        app.mode = Mode::Prompt;
        app.prompt_input = "fix the bug".into();
        let output = render(&mut app);
        assert!(output.contains("prompt (my-app)>"));
        assert!(output.contains("fix the bug"));
    }

    // --- Help overlay ---

    #[test]
    fn help_overlay_shows_keybindings() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
        ]);
        app.mode = Mode::Help;
        // Use a taller terminal so all content fits
        let backend = TestBackend::new(120, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| ui(frame, &mut app)).unwrap();
        let output = buffer_to_string(terminal.backend().buffer());
        assert!(output.contains("Keybindings"));
        assert!(output.contains("j / ↓"));
        assert!(output.contains("Cost estimate"));
    }

    // --- Navigation state ---

    #[test]
    fn next_wraps_around() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
            make_session("b:0.0", Status::Idle, "/b"),
        ]);
        assert_eq!(app.list_state.selected(), Some(0));
        app.next();
        assert_eq!(app.list_state.selected(), Some(1));
        app.next();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn previous_wraps_around() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
            make_session("b:0.0", Status::Idle, "/b"),
        ]);
        assert_eq!(app.list_state.selected(), Some(0));
        app.previous();
        assert_eq!(app.list_state.selected(), Some(1));
    }

    // --- Filter logic ---

    #[test]
    fn apply_filter_narrows_list() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/home/user/frontend"),
            make_session("proj:1.0", Status::Idle, "/home/user/backend"),
        ]);
        app.filter_query = "front".into();
        app.apply_filter();
        assert_eq!(app.filtered.len(), 1);
        assert_eq!(app.sessions[app.filtered[0]].cwd, "/home/user/frontend");
    }

    #[test]
    fn apply_filter_matches_session_label() {
        let mut app = test_app(vec![
            make_session("my-project:0.0", Status::Idle, "/a"),
            make_session("other:0.0", Status::Idle, "/b"),
        ]);
        app.filter_query = "my-proj".into();
        app.apply_filter();
        assert_eq!(app.filtered.len(), 1);
    }

    #[test]
    fn apply_filter_empty_shows_all() {
        let mut app = test_app(vec![
            make_session("a:0.0", Status::Idle, "/a"),
            make_session("b:0.0", Status::Idle, "/b"),
        ]);
        app.filter_query = String::new();
        app.apply_filter();
        assert_eq!(app.filtered.len(), 2);
    }

    #[test]
    fn apply_filter_case_insensitive() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/home/user/MyProject"),
        ]);
        app.filter_query = "myproject".into();
        app.apply_filter();
        assert_eq!(app.filtered.len(), 1);
    }

    // --- Approve (select_option) guard logic ---

    #[test]
    fn select_option_does_nothing_on_idle_session() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Idle, "/a"),
        ]);
        let before = app.last_refresh;
        app.select_option(1);
        // last_refresh should NOT be reset since session is idle
        assert_eq!(app.last_refresh, before);
    }

    #[test]
    fn select_option_does_nothing_on_working_session() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Working("Thinking…".into()), "/a"),
        ]);
        let before = app.last_refresh;
        app.select_option(1);
        assert_eq!(app.last_refresh, before);
    }

    #[test]
    fn select_option_does_nothing_when_no_selection() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::WaitingForApproval, "/a"),
        ]);
        app.list_state.select(None);
        let before = app.last_refresh;
        app.select_option(1);
        assert_eq!(app.last_refresh, before);
    }

    // --- Send prompt guard logic ---

    #[test]
    fn send_prompt_does_nothing_on_working_session() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::Working("Thinking…".into()), "/a"),
        ]);
        let before = app.last_refresh;
        app.send_prompt("do something");
        assert_eq!(app.last_refresh, before);
    }

    #[test]
    fn send_prompt_does_nothing_on_approval_session() {
        let mut app = test_app(vec![
            make_session("proj:0.0", Status::WaitingForApproval, "/a"),
        ]);
        let before = app.last_refresh;
        app.send_prompt("do something");
        assert_eq!(app.last_refresh, before);
    }
}
