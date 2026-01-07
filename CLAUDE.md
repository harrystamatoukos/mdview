# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
cargo build --release        # Build optimized binary
cargo test                   # Run all tests
cargo run -- <file>          # Interactive pager mode
cargo run -- --watch <file>  # Watch mode (auto-refresh on changes)
cargo run -- --print <file>  # Print to stdout (for piping)
```

## Architecture

**mdview** is a terminal markdown viewer with WYSIWYG editing. Pipeline architecture:

```
markdown → parser.rs → Elements → renderer.rs → Text + LayoutMap → pager.rs → Terminal
                         ↑                            ↓
                    (with SourceSpan)           (screen ↔ source mapping)
```

### Core Modules

| Module | Purpose |
|--------|---------|
| `parser.rs` | Converts markdown to AST using pulldown-cmark. Every `Element` stores `SourceSpan` (byte offsets) for WYSIWYG |
| `renderer.rs` | Two pipelines: plain text (`render()`) and styled TUI (`render_to_text_mapped()` with position tracking) |
| `pager.rs` | Event loop, keyboard/mouse handling, smooth scrolling. View mode (vim-style) and edit mode |
| `position.rs` | `LayoutMap` for bidirectional screen ↔ source offset mapping. O(log n) lookups after `build_index()` |
| `editor.rs` | Mutable content state. Insert/delete operations at byte offsets |
| `selection.rs` | Anchor/cursor selection tracking |
| `theme.rs` | Color palette and layout constants (OPTIMAL_WIDTH=66, spacing values) |
| `watcher.rs` | File system watcher for `--watch` mode |

### Key Patterns

**Byte Offsets Everywhere**: Cursor positions, selections, source spans are all `usize` byte offsets (never char indices). Helper methods normalize to UTF-8 boundaries.

**Source Position Preservation**: Parser uses `into_offset_iter()` to track byte offsets. Every rendered character maps to `Option<usize>` — `Some(offset)` for real content, `None` for synthetic chrome (margins, borders, bullets).

**MappedLineBuilder**: Builder pattern in renderer accumulates visual spans + position data simultaneously. Use `push_mapped()` for content, `push_synthetic()` for UI chrome.

**Re-render on Edit**: After each edit, pager calls `render_to_text_mapped()` and rebuilds the layout map. Fast enough for reasonable documents.

**Smooth Scrolling**: Exponential easing with target-based animation. Adaptive polling (16ms when animating, 50ms idle).

### Layout Constants (in theme.rs)

- `OPTIMAL_WIDTH`: 66 chars (typographic optimal reading measure)
- `LEFT_MARGIN`: 4, `MIN_MARGIN`: 2
- `HEADING_SPACING_MAJOR`: 3 lines, `HEADING_SPACING_MINOR`: 2
- `BLOCKQUOTE_WIDTH_REDUCTION`: 6

## Conventions

- Use `anyhow` for error handling (application code)
- All positions are byte offsets into source markdown
- `Option<usize>` semantics: `None` = no cursor/synthetic character
- Follow patterns in `RUST_PRINCIPLES.md` for idiomatic Rust
