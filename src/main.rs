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
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    DefaultTerminal, Frame,
};

use ansi_to_tui::IntoText as _;
use tmux::{Session, Status};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

const PREVIEW_SCROLL_STEP: u16 = 10;

enum Mode {
    Normal,
    Filter,
}

struct App {
    sessions: Vec<Session>,
    filtered: Vec<usize>,
    list_state: ListState,
    last_refresh: Instant,
    preview: String,
    preview_scroll: u16,
    mode: Mode,
    filter_query: String,
    changed: HashMap<String, bool>,
    prev_statuses: HashMap<String, Status>,
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
            mode: Mode::Normal,
            filter_query: String::new(),
            changed: HashMap::new(),
            prev_statuses: HashMap::new(),
        })
    }

    fn refresh(&mut self) {
        if self.last_refresh.elapsed() < REFRESH_INTERVAL {
            return;
        }
        let selected_target = self.selected_session().map(|s| s.target.clone());
        self.sessions = tmux::detect_sessions().unwrap_or_default();

        // Detect status changes
        for session in &self.sessions {
            if let Some(prev) = self.prev_statuses.get(&session.target) {
                if *prev != session.status {
                    self.changed.insert(session.target.clone(), true);
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
        self.preview_scroll = 0;
    }

    fn scroll_preview_down(&mut self) {
        let line_count = self.preview.lines().count() as u16;
        self.preview_scroll = self.preview_scroll.saturating_add(PREVIEW_SCROLL_STEP).min(line_count.saturating_sub(1));
    }

    fn scroll_preview_up(&mut self) {
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

    fn approve_selected(&mut self) {
        if let Some(session) = self.selected_session() {
            if session.status == Status::WaitingForApproval {
                let _ = tmux::send_keys(&session.target, "1");
                // Force a refresh on next tick
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
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match app.mode {
                    Mode::Normal => match (key.code, ctrl) {
                        (KeyCode::Char('q'), _) => break,
                        (KeyCode::Char('j'), false) | (KeyCode::Down, false) => app.next(),
                        (KeyCode::Char('k'), false) | (KeyCode::Up, false) => app.previous(),
                        (KeyCode::Enter, _) | (KeyCode::Char('l'), false) => app.jump_to_selected(),
                        (KeyCode::Char('d'), true) => app.scroll_preview_down(),
                        (KeyCode::Char('u'), true) => app.scroll_preview_up(),
                        (KeyCode::Char('a'), false) => app.approve_selected(),
                        (KeyCode::Char('/'), false) => {
                            app.mode = Mode::Filter;
                            app.filter_query.clear();
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
        Mode::Normal => "j/k navigate · l/Enter jump · a approve · / filter · Ctrl+d/u scroll · q quit",
    };
    frame.render_widget(
        Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray)),
        help_bar,
    );

    let title = Paragraph::new(Line::from(" Claude Manager "))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
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
    let preview = Paragraph::new(preview_text)
        .block(
            Block::default()
                .title(preview_title)
                .borders(Borders::ALL),
        )
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview, preview_area);
}
