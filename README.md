# mdview

A beautiful, **read-only** terminal markdown reader. Designed for reading, not editing.

mdview renders markdown the way a book treats text — a comfortable measure, generous
whitespace, and clear typographic hierarchy — and ships two readers:

- **Graphical reader (default).** Lays out the page with a real proportional font
  (via [cosmic-text](https://github.com/pop-os/cosmic-text) — shaping, kerning,
  anti-aliasing) and displays it inline through a terminal **graphics protocol**
  (kitty / iTerm2). It reads like a native app, not a grid of monospace cells.
  Point it at a directory (or run it with no argument) and it opens a **file-tree
  sidebar** to browse and read every markdown file in the tree — with `/` to
  **search** across the tree (by name and contents) and a vim-style **keyboard
  cursor** to select and copy text without the mouse.
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
mdview                        # browse the current directory (file-tree sidebar)
mdview ./docs                 # browse a directory
mdview document.md            # open a file (sidebar lists its siblings)
mdview --tui document.md      # classic text reader (single file)
mdview --print document.md    # render to stdout (for piping)
mdview --watch document.md    # auto-refresh on file changes (text reader)
mdview --theme dark document.md   # text reader theme: paper | dark | light

# Preview the graphical typography as a PNG (no graphics terminal required)
mdview --export-png out.png document.md
```

The directory sidebar is part of the graphical reader; `--tui`, `--print`,
`--watch`, and `--export-png` operate on a single file.

## Keyboard shortcuts

**Reading**

| Key | Action |
|-----|--------|
| `j` `k` / `↑` `↓` | Scroll |
| `Space` / `PageDown` / `PageUp` | Page down / up |
| `g` / `G` (or `Home` / `End`) | Top / bottom |
| Mouse wheel | Scroll |
| `/` | Find files (by name **and** contents) |
| `Tab` | Focus the file sidebar |
| `v` | Start the keyboard cursor (select & copy) |
| `q` / `Esc` | Quit |

**File sidebar** (`Tab` to focus)

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move the selection |
| `Enter` | Open a file, or expand/collapse a folder |
| Click / wheel | Select or scroll |

**Find** (`/`)

| Key | Action |
|-----|--------|
| *type* | Query matched against filenames (fuzzy) and file contents |
| `Enter` | Run the search; results rank filename hits above content hits |
| `↑` / `↓`, `Enter` | Move through results and open one |
| `Esc` | Clear and return to the tree |

**Cursor & selection** — press `v` in the reader to drop a keyboard cursor

| Key | Action |
|-----|--------|
| `←→↑↓` / `h` `j` `k` `l` | Move the cursor (the page follows) |
| `w` / `b` | Move / extend by word |
| `v` | Start / stop selecting |
| `y` / `Enter` | Copy the selection |
| `Esc` | Exit the cursor (back to scrolling) |

The classic `--tui` reader uses the same scroll keys plus `d` / `u` for half-page
scrolling.

## Requirements

- **Graphical reader:** a graphics-capable terminal — **Ghostty, Kitty, iTerm2, or
  WezTerm**. On any other terminal mdview falls back to the text reader.
- **Text reader (`--tui`):** any terminal; truecolor recommended.
- **Building from source:** Rust 1.85+ (edition 2024).

## What it renders

Headings, paragraphs, **bold** / *italic* / `inline code`, links, ordered &
unordered (nested) lists, task lists, footnotes, blockquotes, fenced code blocks,
horizontal rules, tables (tabular when they fit, card layout when too wide), and
native **charts** (see below).

## Charts

A fenced code block tagged `chart` becomes a native chart — no browser, no
external process. The body is a small YAML document. Charts render three ways:

- **Graphical reader** — a real rasterized chart, drawn straight into the page.
- **`--tui` / `--print`** — a text chart: block bars, a unicode sparkline, or a
  ranked list, depending on the type.
- **Any other markdown tool (GitHub, etc.)** — the block falls back to a plain
  code block showing the YAML, which still reads as legible data.

A chart that fails to parse degrades to a code block too, so a deck never breaks.

### Bar

````markdown
```chart
type: bar
title: Weekly active doctors
xlabel: Day
ylabel: Count
x: [Mon, Tue, Wed, Thu, Fri]
y: [12, 19, 14, 22, 30]
```
````

### Line (single or multiple series)

````markdown
```chart
type: line
title: Score over time
xlabel: Week
ylabel: Score
x: [1, 2, 3, 4, 5]
series:
  - name: Praxis
    y: [41, 47, 52, 58, 63]
  - name: Baseline
    y: [40, 41, 42, 41, 43]
```
````

A single series can skip `series:` and give `y:` directly.

### Pie

````markdown
```chart
type: pie
title: Where the week went
data:
  Pipeline: 40
  Evals: 25
  Recruiting: 20
  Meetings: 15
```
````

### Scatter

````markdown
```chart
type: scatter
title: Dose vs response
xlabel: Dose
ylabel: Response
points: [[1, 2], [2, 3.5], [3, 3], [4, 5], [5, 4.5]]
```
````

### Fields

| Field | Applies to | Meaning |
|-------|-----------|---------|
| `type` | all | `bar`, `line`, `scatter`, or `pie`. **Required.** |
| `title` | all | Chart title. Optional. |
| `xlabel`, `ylabel` | bar, line, scatter | Axis titles. Optional. |
| `x` | bar, line | Category labels (text or numbers). |
| `y` | bar, line | Values for a single series. |
| `series` | bar, line | List of `{ name, y }` for multiple series. |
| `points` | scatter | List of `[x, y]` pairs. |
| `data` | pie | Map of label to number (slices keep their order). |

## Notes

- This started as a hands-on project for learning Rust and terminal programming
  (ratatui, raw mode, escape sequences, terminal graphics protocols) — and grew
  into a genuinely pleasant way to read markdown in the terminal.
- The graphical reader transmits each rendered page band to the terminal **once**
  (zlib-compressed) and scrolls by repositioning it, so scrolling stays smooth on
  long documents.

## License

MIT
