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
| `j` / `↓` | Move down in the list   |
| `k` / `↑` | Move up in the list     |
| `q`       | Quit                    |

## What can I do right now?

- See all active Claude Code sessions across your tmux panes
- Browse the session list with `j`/`k` or arrow keys
- Sessions auto-refresh every 2 seconds

## Roadmap

- [x] Detect tmux panes running Claude Code
- [ ] Show task summary and progress per session
- [ ] Detect sessions waiting for input
- [ ] Navigate directly to a session
