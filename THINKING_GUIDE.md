# Thinking Guide: How to Navigate mdview

## The Core Mental Model

Think of mdview as a **coordinate transformer**. Everything flows through these transformations:

```
Source (bytes) ──┬─▶ AST (Elements) ──▶ Visual (Lines) ──▶ Screen (pixels)
                 │                              │
                 └──────────── LayoutMap ◀──────┘
```

**The LayoutMap is the Rosetta Stone** - it's the bidirectional mapping that makes WYSIWYG possible.

---

## The Five Questions

When working on any feature, ask:

### 1. "What coordinate system am I in?"

| If you see... | You're working with... | Range |
|---------------|------------------------|-------|
| `usize` in editor.rs | Source byte offsets | `0..content.len()` |
| `(line, col)` tuple | Screen positions | `0..total_lines`, `0..width` |
| `SourceSpan` | Source byte range | Start/end byte offsets |
| `scroll_current` | Fractional line index | `0.0..total_lines as f64` |

### 2. "Which module owns this responsibility?"

| Responsibility | Module | Key Type |
|----------------|--------|----------|
| "What does the markdown say?" | parser.rs | `Element`, `Span` |
| "What byte offset is the cursor at?" | editor.rs | `EditorState.cursor` |
| "What does it look like on screen?" | renderer.rs | `Text`, `LayoutMap` |
| "How does the user interact?" | pager.rs | `Pager` |
| "What visual styles apply?" | theme.rs | `Theme` |

### 3. "Do I need to re-render?"

Re-rendering is triggered by `pager.re_render()`. You need it when:
- ✅ Content changed (insert, delete, paste)
- ✅ Terminal resized
- ❌ Cursor moved (layout already knows positions)
- ❌ Scroll changed (just viewport shift)
- ❌ Selection changed (highlighted at draw time)

### 4. "Am I crossing an abstraction boundary?"

The clean boundaries are:

```
     parser.rs              renderer.rs              pager.rs
    ┌──────────┐           ┌───────────┐           ┌──────────┐
    │ Markdown │──parse──▶│   AST     │──render──▶│  Visual  │
    │  String  │           │ Elements  │           │   Text   │
    └──────────┘           └───────────┘           └──────────┘
                                                        │
    ┌──────────┐                                        │
    │  Editor  │◀─────────── cursor sync ──────────────┘
    │  State   │
    └──────────┘
```

**Avoid reaching across**. If pager.rs needs parsed info, it should go through the AST, not re-parse.

### 5. "What happens on the next frame?"

The event loop is in `pager.rs:run_event_loop()`:

```rust
loop {
    // 1. Update animation state
    let animating = pager.update_animation();

    // 2. Draw current state
    terminal.draw(|frame| draw(frame, pager, viewport_height))?;

    // 3. Poll for events (adaptive timeout)
    let timeout = if animating { 16ms } else { 50ms };
    if event::poll(timeout)? {
        // 4. Handle event
        match event::read()? { ... }
    }
}
```

**The draw happens BEFORE event handling**. So if you modify state, the user sees it on the *next* frame.

---

## Common Patterns

### Pattern: Position Lookup

```rust
// Screen → Source (for mouse clicks)
layout_map.screen_to_source_nearest(line, col) -> Option<usize>

// Source → Screen (for cursor rendering)
layout_map.source_to_screen(byte_offset) -> Option<(line, col)>
```

### Pattern: Safe Byte Boundary

UTF-8 means not every byte is a valid position. Always use:

```rust
// In editor.rs
fn ensure_char_boundary(&self, pos: usize) -> usize

// Or the helpers
fn prev_char_boundary(&self, pos: usize) -> usize
fn next_char_boundary(&self, pos: usize) -> usize
```

### Pattern: Edit + Re-render

```rust
// 1. Make the edit (updates content + cursor)
self.editor.insert_char(ch);

// 2. Sync pager cursor from editor
self.cursor_source = Some(self.editor.cursor());

// 3. Sync selection
self.selection.move_to(self.editor.cursor());

// 4. Re-render (rebuilds layout_map)
self.re_render();
```

### Pattern: Scroll + Ensure Visible

```rust
// After cursor movement, make sure it's on screen
self.ensure_cursor_visible_with_viewport(viewport_height);

// This adjusts scroll_target if cursor would be off-screen
```

---

## Gotchas

### 1. Byte Offsets Are Not Character Indices

```rust
let s = "Hello 世界";
// "世" starts at byte 6, not character 6
// "界" starts at byte 9, not character 7

// WRONG: s.chars().nth(offset)
// RIGHT: s[byte_offset..].chars().next()
```

### 2. LayoutMap Must Be Rebuilt After Edits

```rust
// After any edit to content:
self.re_render();  // This calls render_to_text_mapped() + build_index()

// The old layout_map is INVALID after content changes
```

### 3. Synthetic vs Mapped Characters

In LayoutMap, characters are either:
- **Mapped**: Have a `source_offset` - real content from the markdown
- **Synthetic**: `source_offset = None` - UI chrome (margins, borders, bullets)

```rust
// In renderer.rs
builder.push_synthetic("  •  ", style);  // No source mapping
builder.push_mapped(text, offset, style); // Has source mapping
```

### 4. Selection is Source Offsets, Not Screen Positions

```rust
pub struct Selection {
    pub anchor: usize,  // Source byte offset where selection started
    pub cursor: usize,  // Source byte offset where selection ends
}

// To highlight on screen, convert via layout_map
let screen_positions = layout_map.source_range_to_screen(start, end);
```

### 5. The Pager Has Multiple "Cursor" Concepts

```rust
// Source cursor (byte offset in markdown)
cursor_source: Option<usize>

// Visual override (used after Enter, before first keystroke)
cursor_visual_override: Option<(usize, usize)>

// Pending paragraph (user clicked in empty space)
pending_paragraph: Option<(usize, usize, usize)>
```

Check `cursor_screen_position()` to see how they're prioritized.

---

## Debugging Tips

### "Where is the cursor really?"

```rust
// Add to draw() temporarily:
let debug = format!(
    "src:{:?} screen:{:?} sel:{}-{}",
    pager.cursor_source,
    pager.cursor_screen_position(),
    pager.selection.start(),
    pager.selection.end()
);
```

### "What's in the LayoutMap?"

```rust
// Dump a line's mappings:
if let Some(chars) = layout_map.line(line_num) {
    for mc in chars {
        eprintln!("{:?} -> {:?}", mc.ch, mc.source_offset);
    }
}
```

### "Is the index built?"

The LayoutMap has an `index_built` flag. If you're getting `None` from lookups:
```rust
layout_map.build_index();  // Must call after pushing all lines
```

---

## File-by-File Summary

| File | Lines | One-Line Summary |
|------|-------|------------------|
| `main.rs` | 58 | CLI entry, mode selection |
| `parser.rs` | 489 | Markdown → AST with source spans |
| `renderer.rs` | 1429 | AST → styled terminal output + layout map |
| `pager.rs` | 1767 | Event loop, input handling, state management |
| `editor.rs` | 894 | Content mutations, cursor, undo/redo |
| `position.rs` | 908 | Coordinate mapping primitives |
| `selection.rs` | 211 | Anchor/cursor selection model |
| `theme.rs` | 298 | Colors, layout constants |

---

## When Adding a New Feature

1. **Draw it first** - sketch what the user sees
2. **Identify the coordinate system** - source bytes? screen positions?
3. **Find the ownership** - which module should own this?
4. **Trace the data flow** - how does info flow from input to output?
5. **Check for re-render needs** - does this change content or just view?
6. **Test boundary conditions** - UTF-8 edges, empty content, selection active
