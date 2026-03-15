use std::fmt;
use std::process::Command;

use color_eyre::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Working(String),
    WaitingForApproval,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Idle => write!(f, "idle"),
            Status::Working(task) => write!(f, "{task}"),
            Status::WaitingForApproval => write!(f, "needs approval"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub target: String,
    pub status: Status,
}

impl Session {
    pub fn label(&self) -> &str {
        self.target.split(':').next().unwrap_or(&self.target)
    }
}

pub fn detect_status(pane_content: &str) -> Status {
    let lines: Vec<&str> = pane_content
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(15)
        .collect();

    // Check for permission/approval prompt
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("1. Yes") || trimmed.starts_with("❯ 1. Yes") {
            return Status::WaitingForApproval;
        }
        if trimmed == "Do you want to proceed?" {
            return Status::WaitingForApproval;
        }
    }

    // Check for working status (spinner words like "Marinating…", "Canoodling…")
    for line in &lines {
        let trimmed = line.trim();
        // Claude Code status lines: "✢ Marinating…", "✶ Canoodling… (46s · ↓ 114 tokens)"
        if trimmed.len() > 2 && trimmed.ends_with('…') || trimmed.contains("… (") {
            let task = trimmed
                .chars()
                .skip_while(|c| !c.is_alphabetic())
                .collect::<String>();
            if !task.is_empty() {
                return Status::Working(task);
            }
        }
        // Also match tool execution lines like "⏺ Bash(…)" or "Running…"
        if trimmed.contains("Running…") {
            return Status::Working("Running…".to_string());
        }
    }

    Status::Idle
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
            let target = parts[0].to_string();
            let pane_content = capture_pane(&target, 30).unwrap_or_default();
            let status = detect_status(&pane_content);
            Some(Session { target, status })
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

/// Switch the current tmux client to the given target pane.
/// target format: "session:window.pane" e.g. "0-gov:2.1"
pub fn switch_to_pane(target: &str) -> Result<()> {
    // tmux select-pane/select-window with the full target is enough
    // but switch-client needs just the session name
    let session = target.split(':').next().unwrap_or(target);

    Command::new("tmux")
        .args(["switch-client", "-t", session])
        .output()?;
    // select-pane with the full target selects both the window and pane
    Command::new("tmux")
        .args(["select-pane", "-t", target])
        .output()?;

    Ok(())
}
