# Architecture Review: Better Primitives for mdview

## Executive Summary

The codebase has solid foundations but is reaching a complexity threshold where better primitives will unlock cleaner thinking. The main issues:

1. **pager.rs is a 1700-line God Object** - it handles scrolling, editing, selection, mouse, keyboard, rendering
2. **Primitive obsession** - byte offsets, screen positions, and indices are all `usize`
3. **State coupling** - editing state, view state, and UI state are tangled together
4. **Duplicated rendering** - two nearly-identical render pipelines

## Core Insight: The Position Problem

The most fundamental issue is **positional confusion**. The codebase deals with multiple coordinate systems:

| What | Current Type | Problem |
|------|--------------|---------|
| Source byte offset | `usize` | No type distinction |
| Screen position | `(usize, usize)` | Tuple ordering unclear |
| Character index | `usize` | Confused with byte offset |
| Scroll position | `f64` | Different unit than others |

When everything is `usize`, it's easy to accidentally pass a screen column where a byte offset is expected.

---

## Recommended Primitives

### 1. `ByteOffset` - Source Position Primitive

```rust
/// Byte offset into the source markdown file
///
/// This is the universal coordinate system for editing operations.
/// All cursor positions, selections, and source spans use this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteOffset(pub usize);

impl ByteOffset {
    pub const ZERO: ByteOffset = ByteOffset(0);

    pub fn advance(self, bytes: usize) -> Self {
        ByteOffset(self.0 + bytes)
    }

    pub fn retreat(self, bytes: usize) -> Self {
        ByteOffset(self.0.saturating_sub(bytes))
    }
}

// Enables `offset + 1` syntax
impl std::ops::Add<usize> for ByteOffset { ... }
impl std::ops::Sub<usize> for ByteOffset { ... }
```

**Why**: Makes it impossible to accidentally pass a screen column where a byte offset is expected. The type name documents intent at every call site.

### 2. `ScreenPos` - Visual Position Primitive

```rust
/// Position on the rendered screen (line, column)
///
/// Line 0 is the first visible line. Column 0 is the leftmost.
/// This is purely visual - doesn't account for scroll offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenPos {
    pub line: usize,
    pub col: usize,
}

impl ScreenPos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}
```

**Why**: Named fields prevent (col, line) vs (line, col) bugs. Clear documentation of coordinate system.

### 3. `Span<T>` - Generic Range Primitive

```rust
/// A range with start/end of type T
///
/// More ergonomic than std::ops::Range for our use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span<T> {
    pub start: T,
    pub end: T,
}

// Type aliases for common uses
pub type SourceSpan = Span<ByteOffset>;
pub type ScreenSpan = Span<ScreenPos>;
```

**Why**: The current `SourceSpan` is fine but this generalizes nicely. Having `ScreenSpan` for selection highlighting would be cleaner.

---

## Recommended Abstractions

### 4. `Document` - Content + Metadata Bundle

Currently content lives in `EditorState` but parsing happens in `renderer`. The document abstraction bundles these:

```rust
/// A markdown document with its parsed representation
pub struct Document {
    /// Raw markdown source
    source: String,
    /// Parsed AST (lazily recomputed on edit)
    elements: Vec<Element>,
    /// Whether elements are stale (need re-parse)
    dirty: bool,
}

impl Document {
    pub fn source(&self) -> &str { &self.source }
    pub fn elements(&self) -> &[Element] { ... } // lazily re-parses if dirty
    pub fn edit(&mut self, edit: Edit) { ... }
}
```

**Why**: Single source of truth for content. Lazy parsing means we only pay parse cost when rendering.

### 5. `Cursor` - Position + Movement Logic

Extract cursor logic from Pager:

```rust
/// Editing cursor with source position and navigation
pub struct Cursor {
    /// Current position in source
    position: ByteOffset,
    /// Preferred column for vertical movement (sticky column)
    preferred_col: Option<usize>,
}

impl Cursor {
    pub fn move_right(&mut self, layout: &LayoutMap) { ... }
    pub fn move_left(&mut self, layout: &LayoutMap) { ... }
    pub fn move_up(&mut self, layout: &LayoutMap) { ... }
    pub fn move_down(&mut self, layout: &LayoutMap) { ... }
    pub fn word_left(&mut self, doc: &Document) { ... }
    pub fn word_right(&mut self, doc: &Document) { ... }
}
```

**Why**: Movement logic is currently scattered through 200+ lines in pager.rs. This encapsulates it.

### 6. `ViewState` - Scroll + Viewport

Extract view state from Pager:

```rust
/// Viewport and scroll state for the pager
pub struct ViewState {
    /// Target scroll position (for animation)
    scroll_target: f64,
    /// Current animated scroll position
    scroll_current: f64,
    /// Viewport dimensions
    viewport: Rect,
}

impl ViewState {
    pub fn scroll_up(&mut self, lines: usize) { ... }
    pub fn scroll_down(&mut self, lines: usize) { ... }
    pub fn scroll_to(&mut self, line: usize) { ... }
    pub fn animate(&mut self) -> bool { ... } // returns true if still animating
    pub fn visible_lines(&self) -> Range<usize> { ... }
}
```

**Why**: Scroll animation logic is 50+ lines that doesn't need to live in the main Pager struct.

### 7. `InputState` - Mouse + Click Tracking

Extract input state:

```rust
/// Tracks input state for click detection and drag selection
pub struct InputState {
    /// For double/triple click detection
    last_click: Option<(Instant, ScreenPos)>,
    click_count: u8,
    /// Mouse drag state
    dragging: bool,
    /// Last input time (for responsive polling)
    last_input: Instant,
}

impl InputState {
    pub fn record_click(&mut self, pos: ScreenPos) -> ClickType { ... }
    pub fn start_drag(&mut self) { ... }
    pub fn is_dragging(&self) -> bool { ... }
}

pub enum ClickType { Single, Double, Triple }
```

**Why**: Click detection logic is ~40 lines that obscure the main event handling.

---

## Structural Changes

### 8. Split `pager.rs`

The Pager struct has **20+ fields** and **60+ methods**. Suggested split:

```
src/
  pager/
    mod.rs          # Pager struct and run loop
    view.rs         # ViewState
    input.rs        # InputState
    handlers.rs     # Key/mouse event handlers
    draw.rs         # draw() function
```

The Pager becomes a thin coordinator:

```rust
pub struct Pager {
    document: Document,
    cursor: Cursor,
    selection: Selection,
    view: ViewState,
    input: InputState,
    layout: LayoutMap,
    theme: Theme,
}
```

### 9. Unify Rendering Pipelines

Currently `render_to_text` and `render_to_text_mapped` share ~80% of their code. Options:

**Option A**: Single function with flag
```rust
pub fn render(content: &str, width: u16, theme: &Theme, track_positions: bool)
    -> (Text<'static>, Option<LayoutMap>)
```

**Option B**: Builder pattern
```rust
Renderer::new(content, width, theme)
    .with_position_tracking()
    .build()
```

**Option C** (recommended): Always track positions, it's cheap enough
```rust
pub fn render(content: &str, width: u16, theme: &Theme)
    -> (Text<'static>, LayoutMap)
```

---

## Implementation Priority

### Phase 1: Type Safety (Low Risk, High Value)
1. Introduce `ByteOffset` newtype
2. Introduce `ScreenPos` struct
3. Update `SourceSpan` to use `ByteOffset`
4. Update signatures throughout codebase

### Phase 2: Extract ViewState (Medium Risk)
1. Create `ViewState` struct
2. Move scroll logic out of Pager
3. Update event loop

### Phase 3: Extract InputState (Medium Risk)
1. Create `InputState` struct
2. Move click detection logic
3. Update event handlers

### Phase 4: Extract Cursor (Higher Risk)
1. Create `Cursor` struct with movement methods
2. Requires careful coordination with LayoutMap
3. Test thoroughly

### Phase 5: Split pager.rs (Refactor)
1. Create pager/ directory
2. Move handlers to separate file
3. Move draw to separate file

---

## Questions for the Engineer

Before implementing, consider:

1. **Is `--watch` mode important?** If so, Document needs to support external reload.

2. **Will you add persistence (saving)?** If so, Document needs dirty tracking and save().

3. **Multiple cursor support?** If planned, Cursor should be designed for it now.

4. **Undo granularity?** Current coalescing is time-based. Word-based might be better.

---

## Conclusion

The current architecture works but will become harder to extend. The key insight is that **type safety is documentation** - when you see `ByteOffset` in a signature, you know exactly what coordinate system you're in.

Start with Phase 1 (newtypes). It's low risk and immediately makes the code more readable. The compiler will catch any mistakes during the migration.
