# Claude Manager

A terminal dashboard for monitoring and navigating your Claude Code sessions.

Built with [Ratatui](https://ratatui.rs/) in Rust.

## Usage

```sh
cargo run
```

Press `q` to quit.

## What can I do right now?

- Launch the TUI and see a header + empty sessions panel
- That's it so far — session detection is coming next

## Roadmap

- [ ] Detect tmux panes running Claude Code
- [ ] Show task summary and progress per session
- [ ] Detect sessions waiting for input
- [ ] Navigate directly to a session
