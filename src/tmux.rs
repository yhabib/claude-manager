use std::fmt;
use std::process::Command;

use color_eyre::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Working(String),
    WaitingForApproval,
}

impl Status {
    fn priority(&self) -> u8 {
        match self {
            Status::WaitingForApproval => 0,
            Status::Working(_) => 1,
            Status::Idle => 2,
        }
    }
}

impl Ord for Status {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
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
pub struct GitInfo {
    pub branch: String,
    pub is_worktree: bool,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub target: String,
    pub status: Status,
    pub cwd: String,
    pub git: Option<GitInfo>,
}

impl Session {
    pub fn label(&self) -> &str {
        self.target.split(':').next().unwrap_or(&self.target)
    }

    pub fn short_cwd(&self) -> &str {
        self.cwd
            .rsplit('/')
            .next()
            .unwrap_or(&self.cwd)
    }
}

pub fn detect_git_info(cwd: &str) -> Option<GitInfo> {
    if cwd.is_empty() {
        return None;
    }

    let branch = Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    let is_worktree = Command::new("git")
        .args(["-C", cwd, "rev-parse", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let common = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // If git-common-dir differs from git-dir, it's a worktree
            common != ".git"
        })
        .unwrap_or(false);

    Some(GitInfo { branch, is_worktree })
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
        if trimmed.contains("1. Yes") || trimmed.contains("Do you want to proceed?") {
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
            "#{session_name}:#{window_index}.#{pane_index}\t#{pane_title}\t#{pane_current_path}",
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
            let cwd = if parts.len() >= 3 { parts[2].to_string() } else { String::new() };
            let pane_content = capture_pane_plain(&target).unwrap_or_default();
            let status = detect_status(&pane_content);
            let git = detect_git_info(&cwd);
            Some(Session { target, status, cwd, git })
        })
        .collect();

    let mut sessions = sessions;
    sessions.sort_by(|a, b| a.status.cmp(&b.status));

    Ok(sessions)
}

fn capture_pane_plain(target: &str) -> Result<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", target])
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn capture_pane(target: &str) -> Result<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-e", "-t", target])
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

pub fn notify(message: &str) -> Result<()> {
    Command::new("tmux")
        .args(["display-message", message])
        .output()?;
    // Also send a bell to trigger tmux activity alerts
    Command::new("tmux")
        .args(["run-shell", "printf '\\a'"])
        .output()?;
    Ok(())
}

pub fn send_keys(target: &str, keys: &str) -> Result<()> {
    Command::new("tmux")
        .args(["send-keys", "-t", target, keys, "Enter"])
        .output()?;
    Ok(())
}
