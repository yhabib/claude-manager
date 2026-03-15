mod tmux;

use std::io;
use std::time::{Duration, Instant};

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
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

struct App {
    sessions: Vec<Session>,
    list_state: ListState,
    last_refresh: Instant,
    preview: String,
}

impl App {
    fn new() -> Result<Self> {
        let sessions = tmux::detect_sessions().unwrap_or_default();
        let mut list_state = ListState::default();
        if !sessions.is_empty() {
            list_state.select(Some(0));
        }
        Ok(Self {
            sessions,
            list_state,
            last_refresh: Instant::now(),
            preview: String::new(),
        })
    }

    fn refresh(&mut self) {
        if self.last_refresh.elapsed() < REFRESH_INTERVAL {
            return;
        }
        let selected_target = self.selected_session().map(|s| s.target.clone());
        self.sessions = tmux::detect_sessions().unwrap_or_default();

        // Preserve selection by target, or clamp to bounds
        let new_index = selected_target
            .and_then(|t| self.sessions.iter().position(|s| s.target == t))
            .or(if self.sessions.is_empty() { None } else { Some(0) });
        self.list_state.select(new_index);
        self.refresh_preview();
        self.last_refresh = Instant::now();
    }

    fn refresh_preview(&mut self) {
        self.preview = self
            .selected_session()
            .and_then(|s| tmux::capture_pane(&s.target).ok())
            .unwrap_or_default();
    }

    fn selected_session(&self) -> Option<&Session> {
        self.list_state.selected().and_then(|i| self.sessions.get(i))
    }

    fn next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % self.sessions.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
        self.refresh_preview();
    }

    fn previous(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) | None => self.sessions.len() - 1,
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
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('j') | KeyCode::Down => app.next(),
                    KeyCode::Char('k') | KeyCode::Up => app.previous(),
                    KeyCode::Enter | KeyCode::Char('l') => app.jump_to_selected(),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn ui(frame: &mut Frame, app: &mut App) {
    let [header, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(frame.area());

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

    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(body);

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .map(|s| {
            let (indicator, indicator_color) = match &s.status {
                Status::Idle => ("●", Color::DarkGray),
                Status::Working(_) => ("◉", Color::Cyan),
                Status::WaitingForApproval => ("⚠", Color::Yellow),
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{indicator} "),
                    Style::default().fg(indicator_color),
                ),
                Span::styled(
                    s.label().to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", s.status),
                    Style::default().fg(indicator_color),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

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
    let preview = Paragraph::new(preview_text).block(
        Block::default()
            .title(preview_title)
            .borders(Borders::ALL),
    );

    frame.render_widget(preview, preview_area);
}
