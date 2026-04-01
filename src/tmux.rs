use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::time::SystemTime;

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
    pub pinned: bool,
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
    // Convert cwd to Claude's project dir format: /Users/foo.bar/baz -> -Users-foo-bar-baz
    let project_dir: String = cwd.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
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

    match jsonl_path {
        Some(path) => read_usage_from_file(&path),
        None => TokenUsage::default(),
    }
}

/// Read aggregated token usage for today and this month across all projects.
/// Uses the timestamp field inside each JSONL message for accurate date filtering.
/// Returns (daily, monthly) totals.
pub fn read_period_usage() -> (TokenUsage, TokenUsage) {
    let home = std::env::var("HOME").unwrap_or_default();
    let projects_dir = format!("{home}/.claude/projects");

    let now = SystemTime::now();
    let secs_since_epoch = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    let secs_in_day = secs_since_epoch % 86400;
    let today_start = now - std::time::Duration::from_secs(secs_in_day);
    let today_str = timestamp_date_str(today_start);
    // First day of the current month (e.g. "2026-04-01")
    let month_str = format!("{}-01", &today_str[..7]);
    let day_of_month: u64 = today_str[8..10].parse().unwrap_or(1);
    let month_start = today_start - std::time::Duration::from_secs((day_of_month - 1) * 86400);

    let mut daily = TokenUsage::default();
    let mut monthly = TokenUsage::default();

    let Ok(projects) = fs::read_dir(&projects_dir) else {
        return (daily, monthly);
    };

    for project in projects.filter_map(|e| e.ok()) {
        if !project.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(files) = fs::read_dir(project.path()) else { continue };
        for entry in files.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "jsonl") {
                continue;
            }
            // Skip files not modified since month start (quick pre-filter)
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
            if modified.is_some_and(|m| m < month_start) {
                continue;
            }

            read_usage_by_period(&path, &today_str, &month_str, &mut daily, &mut monthly);
        }
    }

    (daily, monthly)
}

/// Format a SystemTime as "YYYY-MM-DD" for string comparison with ISO timestamps.
fn timestamp_date_str(time: SystemTime) -> String {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    // Simple date calculation from epoch seconds
    let days = secs / 86400;
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    for (i, &d) in month_days.iter().enumerate() {
        if remaining < d as i64 { m = i + 1; break; }
        remaining -= d as i64;
    }
    format!("{y:04}-{m:02}-{:02}", remaining + 1)
}

fn read_usage_by_period(
    path: &std::path::Path,
    today_str: &str,
    month_str: &str,
    daily: &mut TokenUsage,
    monthly: &mut TokenUsage,
) {
    let Ok(file) = fs::File::open(path) else { return };
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if val.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(ts) = val.get("timestamp").and_then(|t| t.as_str()) else { continue };

        // Compare date portion of ISO timestamp (e.g. "2026-03-22T...")
        if ts < month_str {
            continue;
        }

        let Some(u) = val.pointer("/message/usage") else { continue };
        let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_read = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cache_write = u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

        monthly.input += input;
        monthly.output += output;
        monthly.cache_read += cache_read;
        monthly.cache_write += cache_write;

        if ts >= today_str {
            daily.input += input;
            daily.output += output;
            daily.cache_read += cache_read;
            daily.cache_write += cache_write;
        }
    }
}

fn read_usage_from_file(path: &std::path::Path) -> TokenUsage {
    let Ok(file) = fs::File::open(path) else {
        return TokenUsage::default();
    };
    let mut usage = TokenUsage::default();
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if !line.contains("\"usage\"") {
            continue;
        }
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
        || command == "claude"
        || command.chars().next().is_some_and(|c| c.is_ascii_digit())
            && command.contains('.')
            && command.len() <= 10
}

fn pinned_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".config/claude-manager/pinned")
}

pub fn load_pinned() -> Vec<String> {
    match fs::read_to_string(pinned_path()) {
        Ok(content) => content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => vec![],
    }
}

pub fn save_pinned(targets: &[String]) {
    let path = pinned_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, targets.join("\n"));
}

pub fn list_all_panes() -> Result<Vec<(String, String)>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_path}",
        ])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let panes = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let target = parts.next()?.to_string();
            let cwd = parts.next().unwrap_or("").to_string();
            Some((target, cwd))
        })
        .collect();
    Ok(panes)
}

/// Resolve the tmux pane that launched the manager (via $TMUX_PANE)
/// into a `session:window.pane` target string.
pub fn origin_pane_target() -> Option<String> {
    let pane_id = std::env::var("TMUX_PANE").ok()?;
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-t",
            &pane_id,
            "-p",
            "#{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()?;
    let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if target.is_empty() { None } else { Some(target) }
}

/// Detect all Claude sessions. When `full` is true, also fetch git info and
/// token usage (expensive). When false, only refresh statuses (fast path).
pub fn detect_sessions(full: bool) -> Result<Vec<Session>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}\t#{pane_title}\t#{pane_current_path}\t#{pane_current_command}",
        ])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Build a map of all pane cwds from the same list-panes output (avoids a second call)
    let all_panes: std::collections::HashMap<String, String> = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() >= 3 {
                Some((parts[0].to_string(), parts[2].to_string()))
            } else {
                None
            }
        })
        .collect();

    let mut sessions: Vec<Session> = stdout
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
            let git = if full { detect_git_info(&cwd) } else { None };
            let tokens = if full { read_token_usage(&cwd) } else { TokenUsage::default() };
            Some(Session { target, status, cwd, git, tokens, pinned: false })
        })
        .collect();

    // Also include pinned non-Claude sessions
    let claude_targets: std::collections::HashSet<String> =
        sessions.iter().map(|s| s.target.clone()).collect();
    for target in load_pinned() {
        if !claude_targets.contains(&target) {
            let cwd = all_panes.get(&target).cloned().unwrap_or_default();
            let git = if full { detect_git_info(&cwd) } else { None };
            sessions.push(Session {
                target,
                status: Status::Idle,
                cwd,
                git,
                tokens: TokenUsage::default(),
                pinned: true,
            });
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    // --- Status ordering ---

    #[test]
    fn status_priority_approval_first() {
        assert!(Status::WaitingForApproval < Status::Working("test".into()));
        assert!(Status::Working("test".into()) < Status::Idle);
        assert!(Status::WaitingForApproval < Status::Idle);
    }

    #[test]
    fn status_display() {
        assert_eq!(Status::Idle.to_string(), "idle");
        assert_eq!(Status::WaitingForApproval.to_string(), "needs approval");
        assert_eq!(Status::Working("Reasoning…".into()).to_string(), "Reasoning…");
    }

    // --- Session helpers ---

    #[test]
    fn session_label_extracts_session_name() {
        let s = Session {
            target: "my-project:2.1".into(),
            status: Status::Idle,
            cwd: "/home/user/code".into(),
            git: None,
            tokens: TokenUsage::default(),
            pinned: false,
        };
        assert_eq!(s.label(), "my-project");
    }

    #[test]
    fn session_label_no_colon() {
        let s = Session {
            target: "simple".into(),
            status: Status::Idle,
            cwd: String::new(),
            git: None,
            tokens: TokenUsage::default(),
            pinned: false,
        };
        assert_eq!(s.label(), "simple");
    }

    #[test]
    fn short_cwd_returns_last_segment() {
        let s = Session {
            target: "t:0.0".into(),
            status: Status::Idle,
            cwd: "/home/user/my-project".into(),
            git: None,
            tokens: TokenUsage::default(),
            pinned: false,
        };
        assert_eq!(s.short_cwd(), "my-project");
    }

    #[test]
    fn short_cwd_no_slash() {
        let s = Session {
            target: "t:0.0".into(),
            status: Status::Idle,
            cwd: "just-a-name".into(),
            git: None,
            tokens: TokenUsage::default(),
            pinned: false,
        };
        assert_eq!(s.short_cwd(), "just-a-name");
    }

    // --- detect_status ---

    #[test]
    fn detect_status_idle_on_empty() {
        assert_eq!(detect_status(""), Status::Idle);
    }

    #[test]
    fn detect_status_idle_on_prompt() {
        let content = "Some output\n\n❯ \n";
        assert_eq!(detect_status(content), Status::Idle);
    }

    #[test]
    fn detect_status_waiting_for_approval_yes() {
        let content = "Do you want to proceed?\n\n  > 1. Yes\n    2. Yes, and don't ask again\n    3. No\n";
        assert_eq!(detect_status(content), Status::WaitingForApproval);
    }

    #[test]
    fn detect_status_waiting_for_approval_option_only() {
        let content = "Some text\n  1. Yes\n  2. No\n";
        assert_eq!(detect_status(content), Status::WaitingForApproval);
    }

    #[test]
    fn detect_status_working_spinner() {
        let content = "Previous output\n\n✢ Marinating…\n";
        assert!(matches!(detect_status(content), Status::Working(t) if t == "Marinating…"));
    }

    #[test]
    fn detect_status_working_spinner_with_tokens() {
        let content = "Output\n✶ Canoodling… (46s · ↓ 114 tokens)\n";
        assert!(matches!(detect_status(content), Status::Working(t) if t.starts_with("Canoodling")));
    }

    #[test]
    fn detect_status_working_running() {
        let content = "Some text\nRunning…\n";
        assert_eq!(detect_status(content), Status::Working("Running…".into()));
    }

    #[test]
    fn detect_status_approval_takes_priority_over_working() {
        let content = "✢ Thinking…\n\n  1. Yes\n  2. No\n";
        assert_eq!(detect_status(content), Status::WaitingForApproval);
    }

    // --- TokenUsage ---

    #[test]
    fn token_usage_cost_zero() {
        let usage = TokenUsage::default();
        assert_eq!(usage.estimated_cost(), 0.0);
    }

    #[test]
    fn token_usage_cost_calculation() {
        let usage = TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
        };
        // $5 + $25 + $0.50 + $6.25 = $36.75
        assert!((usage.estimated_cost() - 36.75).abs() < 0.001);
    }

    #[test]
    fn token_usage_cost_output_dominates() {
        let usage = TokenUsage {
            input: 0,
            output: 100_000,
            cache_read: 0,
            cache_write: 0,
        };
        // 100k output * $25/MTok = $2.50
        assert!((usage.estimated_cost() - 2.5).abs() < 0.001);
    }

    // --- is_claude_pane ---

    #[test]
    fn is_claude_pane_by_title() {
        let line = "session:1.0\t✳ Claude Code\t/home/user/project\tzsh";
        assert!(is_claude_pane(line));
    }

    #[test]
    fn is_claude_pane_by_version_command() {
        let line = "session:1.0\t✳ Understand something\t/home/user/project\t2.1.81";
        assert!(is_claude_pane(line));
    }

    #[test]
    fn is_claude_pane_by_claude_command() {
        let line = "blog:1.1\t✳ Fix blog landing page\t/root/blog\tclaude";
        assert!(is_claude_pane(line));
    }

    #[test]
    fn is_claude_pane_not_matching() {
        let line = "session:1.0\tmy terminal\t/home/user\tzsh";
        assert!(!is_claude_pane(line));
    }

    #[test]
    fn is_claude_pane_too_few_fields() {
        let line = "session:1.0\ttitle only";
        assert!(!is_claude_pane(line));
    }

    #[test]
    fn is_claude_pane_long_command_not_version() {
        let line = "session:1.0\ttitle\t/path\tsome-long-command-name";
        assert!(!is_claude_pane(line));
    }

    // --- timestamp_date_str ---

    #[test]
    fn timestamp_date_str_epoch() {
        let time = SystemTime::UNIX_EPOCH;
        assert_eq!(timestamp_date_str(time), "1970-01-01");
    }

    #[test]
    fn timestamp_date_str_known_date() {
        // 2026-03-15 00:00:00 UTC = 1773532800 seconds since epoch
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1773532800);
        assert_eq!(timestamp_date_str(time), "2026-03-15");
    }

    #[test]
    fn timestamp_date_str_leap_year() {
        // 2024-02-29 00:00:00 UTC = 1709164800 seconds since epoch
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1709164800);
        assert_eq!(timestamp_date_str(time), "2024-02-29");
    }

    #[test]
    fn timestamp_date_str_end_of_year() {
        // 2025-12-31 00:00:00 UTC = 1767139200 seconds since epoch
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1767139200);
        assert_eq!(timestamp_date_str(time), "2025-12-31");
    }
}
