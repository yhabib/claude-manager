# Claude Manager

A terminal dashboard for monitoring and navigating your Claude Code sessions.

Built with [Ratatui](https://ratatui.rs/) in Rust.

## Usage

```sh
cargo run
```

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
- Preview the last 50 lines of the selected session's pane output
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
