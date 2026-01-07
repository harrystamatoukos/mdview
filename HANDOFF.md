# mdview Complete Editor Handoff

## What Was Built

Terminal markdown editor where **you never see markdown syntax**. User types `**bold**`, sees "bold" styled. Markers hidden, editing works. Full WYSIWYG editing with undo/redo, list nesting, and word count.

## Architecture (Critical)

```
Source string → Parser (pulldown-cmark) → AST with SourceSpans → Renderer → Screen + LayoutMap
                                                                              ↓
                                                                    FormattingSpans tracked
```

**Key insight**: pulldown-cmark gives content ranges EXCLUDING markers. `**bold**` yields text "bold" at bytes 2-6. We calculate marker positions from this.

## Files

| File | Role |
|------|------|
| `position.rs` | `FormattingSpan`, `FormattingKind`, `LayoutMap` with boundary queries |
| `renderer.rs` | `wrap_spans_styled()` returns `(segments, formatting_spans)`, styles preserved |
| `pager.rs` | `FormattingState`, smart syntax entry, all shortcuts, smart Enter, Tab/Shift+Tab |
| `theme.rs` | `inline_code()` style with bg+color |
| `editor.rs` | Text manipulation, cursor management, undo/redo history, word count |

## Key Types

```rust
// editor.rs
EditOperation { Insert { pos, text }, Delete { pos, text } }
History { undos, redos, max_size, last_op_time, coalesce_ms }

// position.rs
FormattingKind { Strong, Emphasis, Code, StrongEmphasis, Strikethrough }
FormattingSpan { kind, marker_start, content_start, content_end, marker_end }

// pager.rs
FormattingState { bold_active, italic_active, code_active, pending: String }
ListItemInfo { marker, marker_len, content }
```

## What Works

### Invisible Markdown (Phase 4)
- Bold/italic/code markers hidden in rendered view
- Smart syntax entry: `**` toggles bold, `*`+char toggles italic, backtick toggles code
- Boundary deletion: Backspace at formatted content start removes opening marker

### Formatting Shortcuts (Phase 5)
- Ctrl+B/I/` for bold/italic/code
- Ctrl+1/2/3/0 for heading levels
- Ctrl+K for link insertion

### Smart Line Behavior (Phase 6)
- Smart Enter: List continuation, list exit on empty item, blockquote continuation
- Tab/Shift+Tab: Indent/outdent list items (2 spaces)

### Persistence & Safety (Phase 8)
- Undo/redo with coalescing (Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y)
- History stack with 1000 operation limit
- 500ms coalesce window for rapid keystrokes

### Polish (Phase 9)
- Word count in status bar (updates live)

## Keyboard Shortcuts (Edit Mode)

| Shortcut | Action |
|----------|--------|
| Ctrl+B | Toggle bold |
| Ctrl+I | Toggle italic |
| Ctrl+` | Toggle code |
| Ctrl+1 | Toggle H1 |
| Ctrl+2 | Toggle H2 |
| Ctrl+3 | Toggle H3 |
| Ctrl+0 | Remove heading |
| Ctrl+K | Insert link |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |
| Ctrl+Y | Redo (alternative) |
| Tab | Indent list item |
| Shift+Tab | Outdent list item |
| Enter | Smart enter (continues lists/blockquotes) |

## Smart Behavior

### Smart Enter
| Context | Action |
|---------|--------|
| `- Item\|` + Enter | Creates `- \|` (new list item) |
| `- \|` + Enter | Exits list (removes marker) |
| `1. Item\|` + Enter | Creates `2. \|` (numbered list continues) |
| `> Quote\|` + Enter | Creates `> \|` (continues blockquote) |
| Normal text | Just inserts newline |

### Undo/Redo
- Operations coalesced within 500ms (typing feels like single undo unit)
- Insert and delete operations tracked separately
- Cursor position restored on undo/redo

## Status Bar
```
 EDIT │ 1-20 of 45 [+] 1234w byte:156 │ Esc:view  Ctrl+Q:quit
```
Shows: mode, scroll position, dirty indicator, word count, cursor position, help text

## Gotchas

1. `FormattingSpan::from_content_range()` calculates marker positions by subtracting marker_len from content_start. Works because pulldown-cmark gives us content-only ranges.

2. `wrap_spans_styled()` returns tuple `(Vec<Vec<StyledSegment>>, Vec<FormattingSpan>)`. Callers must push formatting spans to layout_map.

3. Boundary detection uses `layout_map.formatting_at_content_start(offset)` and `formatting_at_content_end(offset)`.

4. Smart syntax entry buffers `*` in `formatting.pending` to distinguish `*` (italic) from `**` (bold).

5. Selection wrapping inserts closing marker first (preserves byte offsets), then opening.

6. Line helpers: `editor.line_start(pos)`, `editor.line_end(pos)`, `editor.line_content(pos)`.

7. Undo coalesces operations within 500ms - consecutive inserts become one undo unit.

## Test Commands

```bash
cargo build --release
cargo test  # 35 tests
./target/release/mdview /tmp/test-editing.md
```

## Tests
- 35 unit tests covering:
  - Editor operations (insert, delete, undo, redo)
  - FormattingSpan boundary detection
  - LayoutMap position mapping
  - Selection management

## Reference Docs

- `~/.claude/plans/mdview-editor-vision.md` - Full roadmap (Phases 1-9)
- `DESIGN_PRINCIPLES.md` - Typography philosophy
- `RUST_PRINCIPLES.md` - Code style guide

## What's Remaining (Optional)

1. **Auto-save** - Save to temp file periodically (Phase 8.1)
2. **Find & Replace** (Ctrl+F, Ctrl+H) - Phase 9.4
3. **Focus Mode** - Dim non-current paragraphs (Phase 9.2)
4. **Incremental Rendering** - For very large documents (Phase 7)
