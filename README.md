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

To run without installing:

```sh
cargo run
```

**Important:** Run it inside tmux — the app uses tmux commands to detect and navigate sessions.

### Recommended setup

Start it in a dedicated tmux session so you can jump to it from anywhere:

```sh
tmux new-session -s claude-manager 'claude-manager'
```

Then add a keybinding to your `~/.tmux.conf` to jump back to it:

```tmux
bind m switch-client -t claude-manager
```

Now `prefix + m` (e.g. `Ctrl+a m`) takes you to the dashboard from any session.

### Keys

| Key       | Action                  |
|-----------|-------------------------|
| `j` / `↓`       | Move down in the list          |
| `k` / `↑`       | Move up in the list            |
| `Enter` / `l`   | Jump to the selected session   |
| `q`             | Quit                           |

## What can I do right now?

- See all active Claude Code sessions across your tmux panes
- Browse the session list with `j`/`k` or arrow keys
- Preview the selected session's pane output with full color
- Press `Enter` or `l` to jump straight into that tmux session
- See at a glance what each session is doing:
  - `●` grey — idle, waiting for your input
  - `◉` cyan — actively working
  - `⚠` yellow — needs approval (permission prompt)
- Sessions auto-refresh every 2 seconds

## Roadmap

- [x] Detect tmux panes running Claude Code
- [x] Preview pane content for selected session
- [x] Navigate directly to a session
- [x] Show task status per session
- [x] Detect sessions waiting for approval

---

Built with [Claude Code](https://claude.ai/claude-code)
