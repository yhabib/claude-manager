use std::io;

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};

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
    loop {
        terminal.draw(ui)?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break,
                _ => {}
            }
        }
    }
    Ok(())
}

fn ui(frame: &mut Frame) {
    let [header, body] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)])
        .areas(frame.area());

    let title = Paragraph::new(Line::from(" Claude Manager "))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));

    let placeholder = Paragraph::new("No sessions detected. Press q to quit.")
        .block(Block::default().title(" Sessions ").borders(Borders::ALL));

    frame.render_widget(title, header);
    frame.render_widget(placeholder, body);
}
