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

| Key              | Action                                      |
|------------------|---------------------------------------------|
| `j` / `↓`       | Move down in the list                       |
| `k` / `↑`       | Move up in the list                         |
| `Enter` / `l`   | Jump to the selected session                |
| `a`             | Approve permission prompt without switching  |
| `/`             | Filter sessions by name or directory         |
| `J` (shift+j)   | Scroll preview down                          |
| `K` (shift+k)   | Scroll preview up                            |
| `q`             | Quit                                         |

**Filter mode:** type to narrow the list, `Enter` to confirm, `Esc` to clear.

## What can I do right now?

- See all active Claude Code sessions across your tmux panes
- Sessions grouped by tmux session name, sorted by priority
- Preview the selected session's pane output with full color
- Press `Enter` or `l` to jump straight into a session
- Press `a` to approve permission prompts without leaving the dashboard
- Filter sessions with `/` — matches session name and working directory
- Scroll the preview with `J` / `K` (shift+j/k)
- See at a glance what each session is doing:
  - `●` grey — idle, waiting for your input
  - `◉` cyan — actively working
  - `⚠` yellow — needs approval (permission prompt)
  - `*` magenta — status changed since you last looked
- Shows the working directory for each session
- Help bar at the bottom with available keys
- Sessions auto-refresh every 2 seconds

---

Built with [Claude Code](https://claude.ai/claude-code)
