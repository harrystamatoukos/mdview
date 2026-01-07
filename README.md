# mdview

A beautiful terminal markdown viewer with editorial aesthetics. Designed for **reading**, not coding.

> **Learning Project**: This project exists primarily as a hands-on way to learn Rust and terminal programming (TUI with ratatui, raw mode, escape sequences, etc.). It's a real, usable tool — but also a playground for understanding how terminals actually work under the hood.

## Philosophy

Most markdown viewers feel like code editors. mdview takes a different approach:

- **66-character line width** — The typographer's optimal reading measure (Bringhurst)
- **Generous whitespace** — Breathing room between sections
- **Warm, muted colors** — Easy on the eyes, like a well-printed book
- **Respects your terminal** — Body text uses your chosen colors

## Installation

```bash
# From source
git clone https://github.com/harrystamatoukos/mdview
cd mdview
cargo build --release

# Copy to your path
cp target/release/mdview ~/.local/bin/
# or
sudo cp target/release/mdview /usr/local/bin/
```

## Usage

```bash
# Interactive pager mode
mdview document.md

# Watch mode — auto-refresh on file changes
mdview -w document.md

# Print to stdout (for piping)
mdview -p document.md
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |
| `d` | Half page down |
| `u` | Half page up |
| `g` | Go to top |
| `G` | Go to bottom |
| `q` | Quit |

## Features

- Headers, paragraphs, lists (ordered & unordered)
- Blockquotes with subtle left border
- Code blocks with syntax highlighting
- Tables with smart rendering:
  - Fits? Traditional tabular layout
  - Too wide? Card layout (each row as vertical block)
- Horizontal rules
- Watch mode for live editing

## Requirements

- Rust 1.85+ (edition 2024)
- A modern terminal with truecolor support (Ghostty, iTerm2, Kitty, etc.)

## License

MIT
