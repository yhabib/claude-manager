use std::process::Command;

use color_eyre::Result;

#[derive(Debug, Clone)]
pub struct Session {
    pub target: String,
    pub title: String,
    pub pid: u32,
}

impl Session {
    pub fn label(&self) -> &str {
        let session_name = self.target.split(':').next().unwrap_or(&self.target);
        session_name
    }
}

pub fn detect_sessions() -> Result<Vec<Session>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}\t#{pane_pid}\t#{pane_title}",
        ])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sessions: Vec<Session> = stdout
        .lines()
        .filter(|line| line.contains("Claude Code"))
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 {
                return None;
            }
            Some(Session {
                target: parts[0].to_string(),
                pid: parts[1].parse().unwrap_or(0),
                title: parts[2].to_string(),
            })
        })
        .collect();

    Ok(sessions)
}

pub fn capture_pane(target: &str, lines: u16) -> Result<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", target, "-S", &format!("-{lines}")])
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn switch_to_pane(target: &str) -> Result<()> {
    let parts: Vec<&str> = target.splitn(2, ':').collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let session = parts[0];
    let window_pane = parts[1]; // e.g. "2.1"

    Command::new("tmux")
        .args(["switch-client", "-t", session])
        .output()?;
    Command::new("tmux")
        .args(["select-window", "-t", &format!("{session}:{window_pane}")])
        .output()?;
    Command::new("tmux")
        .args(["select-pane", "-t", target])
        .output()?;

    Ok(())
}
