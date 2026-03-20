# Claude Manager

A terminal dashboard for monitoring and navigating your Claude Code sessions.

Built with [Ratatui](https://ratatui.rs/) in Rust.

## Requirements

- [Rust](https://rustup.rs/) (1.85+)
- [tmux](https://github.com/tmux/tmux) running with one or more Claude Code sessions

## Install

```sh
git clone https://github.com/YusefFernandez/claude-manager.git
cd claude-manager
cargo install --path .
```

This puts `claude-manager` in your `~/.cargo/bin/` so you can run it from anywhere.

**Important:** Run it inside tmux.

### Recommended setup

Start it in a dedicated tmux session:

```sh
tmux new-session -s claude-manager 'claude-manager'
```

Then add a keybinding to your `~/.tmux.conf` to jump back to it from anywhere:

```tmux
bind m switch-client -t claude-manager
```

Now `Ctrl+b m` takes you to the dashboard.

## Features

### Session detection

The dashboard scans all your tmux panes and picks up any running Claude Code instance. Sessions are grouped by their tmux session name and show the working directory so you can tell them apart. Everything refreshes automatically every 2 seconds.

### Status indicators

Each session shows a color-coded status based on its pane content:

- `●` **idle** — Claude finished and is waiting for your next prompt
- `◉` **working** — Claude is actively processing (shows the current activity like "Marinating…")
- `⚠` **needs approval** — a permission prompt is waiting for your response
- `*` **changed** — the status changed since you last selected this session; clears when you navigate to it

### Live preview

The right panel shows the selected session's pane output with full ANSI color support. It auto-scrolls to the bottom so you always see the latest output. Use `J`/`K` to scroll manually — auto-scroll resumes when you switch sessions.

### Jump to session

Press `l` or `Enter` to switch your tmux client directly to the selected session's window and pane. The dashboard stays running in its own pane so you can come back with `prefix + m`.

### Quick approve

Press `a` to approve a permission prompt without switching to that session. This sends the "Yes" response directly to the pane. Only works when the selected session has a `⚠` status.

### Filter

Press `/` to enter filter mode. Type to narrow the session list — it matches against session names and working directories. Press `Enter` to lock in your filter, `Esc` to clear it.

### Priority sorting

Press `s` to toggle automatic sorting by status priority: sessions needing approval float to the top, then working sessions, then idle ones. Off by default so sessions stay in their natural tmux order.

### Git info

Press `w` to toggle git branch and worktree information. When enabled, each session shows its current branch on a second line, with a `[worktree]` tag if it's a git worktree rather than the main repo. Off by default.

### Notifications

When a session transitions to "needs approval", the dashboard sends a `display-message` to your tmux status bar so you notice even when you're working in another pane.

## Keys

| Key            | Action                          |
|----------------|---------------------------------|
| `j` / `↓`     | Move down                       |
| `k` / `↑`     | Move up                         |
| `l` / `Enter` | Jump to session                 |
| `a`            | Approve selected                |
| `J` / `K`     | Scroll preview down / up        |
| `/`            | Filter sessions                 |
| `s`            | Toggle priority sorting         |
| `w`            | Toggle git / worktree info      |
| `?`            | Help overlay                    |
| `q`            | Quit                            |

---

Built with Claude Code
