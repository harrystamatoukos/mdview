# mdview

A beautiful, **read-only** terminal markdown reader. Designed for reading, not editing.

mdview renders markdown the way a book treats text — a comfortable measure, generous
whitespace, and clear typographic hierarchy — and ships two readers:

- **Graphical reader (default).** Lays out the page with a real proportional font
  (via [cosmic-text](https://github.com/pop-os/cosmic-text) — shaping, kerning,
  anti-aliasing) and displays it inline through a terminal **graphics protocol**
  (kitty / iTerm2). It reads like a native app, not a grid of monospace cells.
- **Classic text reader (`--tui`).** Styled terminal text that works in any terminal.
  Used automatically as a fallback when no graphics-capable terminal is detected.

## Install

### Homebrew (macOS)

```bash
brew install harrystamatoukos/tap/mdview
```

### From source

```bash
git clone https://github.com/harrystamatoukos/mdview
cd mdview
cargo build --release
cp target/release/mdview /usr/local/bin/   # or anywhere on your PATH
```

For a leaner, text-only build without the graphical reader:

```bash
cargo build --release --no-default-features
```

## Usage

```bash
mdview document.md            # graphical reader (default)
mdview --tui document.md      # classic text reader
mdview --print document.md    # render to stdout (for piping)
mdview --watch document.md    # auto-refresh on file changes (text reader)
mdview --theme dark document.md   # text reader theme: paper | dark | light

# Preview the graphical typography as a PNG (no graphics terminal required)
mdview --export-png out.png document.md
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `Space` / `PageDown` | Page down |
| `b` / `PageUp` | Page up |
| `d` / `u` | Half page down / up *(text reader)* |
| `g` / `Home` | Go to top |
| `G` / `End` | Go to bottom |
| Mouse wheel | Scroll |
| `q` / `Esc` | Quit |

## Requirements

- **Graphical reader:** a graphics-capable terminal — **Ghostty, Kitty, iTerm2, or
  WezTerm**. On any other terminal mdview falls back to the text reader.
- **Text reader (`--tui`):** any terminal; truecolor recommended.
- **Building from source:** Rust 1.85+ (edition 2024).

## What it renders

Headings, paragraphs, **bold** / *italic* / `inline code`, links, ordered &
unordered (nested) lists, blockquotes, fenced code blocks, horizontal rules, and
tables (tabular when they fit, card layout when too wide).

## Notes

- This started as a hands-on project for learning Rust and terminal programming
  (ratatui, raw mode, escape sequences, terminal graphics protocols) — and grew
  into a genuinely pleasant way to read markdown in the terminal.
- The graphical reader transmits each rendered page band to the terminal **once**
  (zlib-compressed) and scrolls by repositioning it, so scrolling stays smooth on
  long documents.

## License

MIT
