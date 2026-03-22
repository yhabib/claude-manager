use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
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

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    /// Estimated cost in USD based on Claude Opus 4.6 pricing.
    /// Input: $5/MTok, Output: $25/MTok, Cache read: $0.50/MTok, Cache write: $6.25/MTok
    pub fn estimated_cost(&self) -> f64 {
        (self.input as f64 * 5.0
            + self.output as f64 * 25.0
            + self.cache_read as f64 * 0.5
            + self.cache_write as f64 * 6.25)
            / 1_000_000.0
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
    pub tokens: TokenUsage,
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

/// Read token usage from the most recently modified JSONL session file for a given cwd.
pub fn read_token_usage(cwd: &str) -> TokenUsage {
    let home = std::env::var("HOME").unwrap_or_default();
    // Convert cwd to Claude's project dir format: /Users/foo/bar -> -Users-foo-bar
    let project_dir = cwd.replace('/', "-");
    let dir = format!("{home}/.claude/projects/{project_dir}");

    // Find the most recently modified .jsonl file
    let jsonl_path = match fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
            .map(|e| e.path()),
        Err(_) => return TokenUsage::default(),
    };

    let Some(path) = jsonl_path else {
        return TokenUsage::default();
    };

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return TokenUsage::default(),
    };

    let mut usage = TokenUsage::default();
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        // Quick filter before parsing JSON
        if !line.contains("\"usage\"") {
            continue;
        }
        // Parse just enough to extract usage
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if val.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(u) = val.pointer("/message/usage") else { continue };
        usage.input += u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        usage.output += u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        usage.cache_read += u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        usage.cache_write += u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }

    usage
}

fn is_claude_pane(line: &str) -> bool {
    let parts: Vec<&str> = line.splitn(4, '\t').collect();
    if parts.len() < 4 {
        return false;
    }
    let title = parts[1];
    let command = parts[3];
    // Match by title containing "Claude Code" or by command being a semver-like version (e.g. "2.1.81")
    title.contains("Claude Code")
        || command.chars().next().is_some_and(|c| c.is_ascii_digit())
            && command.contains('.')
            && command.len() <= 10
}

pub fn detect_sessions() -> Result<Vec<Session>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}\t#{pane_title}\t#{pane_current_path}\t#{pane_current_command}",
        ])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sessions: Vec<Session> = stdout
        .lines()
        .filter(|line| is_claude_pane(line))
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() < 3 {
                return None;
            }
            let target = parts[0].to_string();
            let cwd = parts[2].to_string();
            let pane_content = capture_pane_plain(&target).unwrap_or_default();
            let status = detect_status(&pane_content);
            let git = detect_git_info(&cwd);
            let tokens = read_token_usage(&cwd);
            Some(Session { target, status, cwd, git, tokens })
        })
        .collect();

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
        .args(["capture-pane", "-p", "-e", "-S", "-1000", "-t", target])
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Switch the current tmux client to the given target pane.
/// target format: "session:window.pane" e.g. "0-gov:2.1"
pub fn switch_to_pane(target: &str) -> Result<()> {
    // Parse "session:window.pane" into parts
    let session = target.split(':').next().unwrap_or(target);
    // "session:window" for select-window (e.g. "0-gov:2")
    let session_window = target.split('.').next().unwrap_or(target);

    Command::new("tmux")
        .args(["switch-client", "-t", session])
        .output()?;
    Command::new("tmux")
        .args(["select-window", "-t", session_window])
        .output()?;
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

/// Send key sequences to a tmux pane.
/// Each entry in `keys` is a separate argument to `send-keys`.
pub fn send_keys(target: &str, keys: &[&str]) -> Result<()> {
    Command::new("tmux")
        .arg("send-keys")
        .arg("-t")
        .arg(target)
        .args(keys)
        .output()?;
    Ok(())
}

/// Open lazygit in a tmux popup targeting the given working directory.
pub fn open_lazygit(cwd: &str) -> Result<()> {
    Command::new("tmux")
        .args(["display-popup", "-d", cwd, "-w", "90%", "-h", "90%", "-E", "lazygit"])
        .output()?;
    Ok(())
}

/// Select option N in a Claude Code selection prompt.
/// Option 1 is already highlighted by default, so just Enter.
/// Option 2 needs one Down, option 3 needs two Downs, etc.
pub fn select_option(target: &str, option: u8) -> Result<()> {
    for _ in 1..option {
        send_keys(target, &["Down"])?;
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    send_keys(target, &["Enter"])?;
    Ok(())
}
