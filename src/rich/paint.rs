//! Proportional-font document rasterization using cosmic-text + swash.
//!
//! Lays out a parsed markdown document with a real desktop-class text stack
//! (shaping, kerning, anti-aliasing, ligatures) and paints it onto an RGBA
//! page bitmap, applying the typographic principles that drive comfortable
//! long-form reading:
//!
//! * Measure ~66 characters per line
//! * Body leading ~1.55x; tighter (~1.2x) leading for large headings
//! * A modular type scale (~1.25 major third) for instant hierarchy
//! * More vertical space *before* a heading than after it
//! * Generous page margins / whitespace

use cosmic_text::{
    Align, Attrs, Buffer, Color, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight,
};
use image::{imageops, imageops::FilterType, Rgba, RgbaImage};

use crate::parser::{CellAlign, Element, Span, SpanKind};

/// RGB triple (matches our theme palette values).
pub type Rgb = (u8, u8, u8);

// ─────────────────────────────────────────────────────────────────────────
// Document style — all sizes in DEVICE pixels (already scaled for crispness)
// ─────────────────────────────────────────────────────────────────────────

/// Visual configuration for the rendered page.
#[derive(Debug, Clone)]
pub struct DocStyle {
    pub body_font: &'static str,
    pub mono_font: &'static str,
    /// Base body font size in device px.
    pub base_px: f32,
    /// Text column width (the "measure") in device px.
    pub content_px: u32,
    /// Left/right page margin in device px.
    pub margin_px: u32,
    /// Top/bottom page padding in device px.
    pub pad_px: u32,
    // Colors
    pub bg: Rgb,
    pub ink: Rgb,
    pub h: [Rgb; 6],
    pub accent: Rgb,
    pub code: Rgb,
    /// Background chip behind inline `code` and code-block panels.
    pub code_bg: Rgb,
    pub link: Rgb,
    pub chrome: Rgb,
}

impl DocStyle {
    /// Clean light reading style at 2x DPI — classic black text on white.
    ///
    /// Near-black ink on an essentially-white page. High contrast without the
    /// glare of pure `#000` on `#FFF`. Headings are darkest (near-black) and
    /// step down in weight/shade for hierarchy.
    pub fn light() -> Self {
        let base_px = 36.0; // ~18pt at 2x
        DocStyle {
            body_font: "Georgia",
            mono_font: "Menlo",
            base_px,
            // ~33em ≈ ~66 characters for a serif → comfortable measure.
            content_px: (base_px * 33.0) as u32,
            margin_px: (base_px * 1.8) as u32,
            pad_px: (base_px * 1.8) as u32,
            bg: (252, 252, 252),  // #FCFCFC essentially white
            ink: (26, 26, 26),    // #1A1A1A near-black body
            h: [
                (0, 0, 0),     // h1 black
                (26, 26, 26),  // h2
                (45, 45, 45),  // h3
                (70, 70, 70),  // h4
                (95, 95, 95),  // h5
                (95, 95, 95),  // h6 (same, rendered dimmer)
            ],
            accent: (120, 120, 120), // neutral gray for bullets/bars/rules
            code: (70, 70, 70),      // dark gray; mono font also sets it apart
            code_bg: (236, 236, 239), // faint cool-gray chip behind inline code
            link: (26, 95, 180),     // #1A5FB4 classic readable blue
            chrome: (200, 200, 200), // #C8C8C8 light borders
        }
    }

    /// Heading size multiplier on the modular scale (major-third-ish).
    fn heading_scale(level: u8) -> f32 {
        match level {
            1 => 2.10,
            2 => 1.62,
            3 => 1.32,
            4 => 1.15,
            5 => 1.00,
            _ => 0.92,
        }
    }

    /// Vertical space *before* a heading of the given level (device px).
    fn heading_space_before(&self, level: u8) -> u32 {
        let m = match level {
            1 => 1.7,
            2 => 1.5,
            3 => 1.2,
            _ => 1.0,
        };
        (self.base_px * m) as u32
    }

    fn para_space(&self) -> u32 {
        (self.base_px * 0.9) as u32
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Internal span styling
// ─────────────────────────────────────────────────────────────────────────

/// Resolved inline style for one run of text.
#[derive(Clone, Copy)]
struct Run {
    bold: bool,
    italic: bool,
    mono: bool,
    /// Draw a subtle background chip behind this run (inline `code`).
    pill: bool,
    color: Rgb,
}

/// Glyph metadata tag marking a run that wants an inline-code background chip.
const PILL_META: usize = 1;

impl Run {
    fn attrs(&self, st: &DocStyle) -> Attrs<'static> {
        let family = if self.mono {
            Family::Name(st.mono_font)
        } else {
            Family::Name(st.body_font)
        };
        let mut a = Attrs::new()
            .family(family)
            .color(Color::rgb(self.color.0, self.color.1, self.color.2));
        if self.bold {
            a = a.weight(Weight::BOLD);
        }
        if self.italic {
            a = a.style(FontStyle::Italic);
        }
        if self.pill {
            a = a.metadata(PILL_META);
        }
        a
    }
}

/// Flatten inline spans into (text, Run) pairs, resolving emphasis/strong/
/// code/link to concrete weights, styles, and colors.
fn flatten_spans(spans: &[Span], st: &DocStyle, base_color: Rgb) -> Vec<(String, Run)> {
    let body = Run { bold: false, italic: false, mono: false, pill: false, color: base_color };
    let mut out: Vec<(String, Run)> = Vec::new();
    for span in spans {
        match &span.kind {
            SpanKind::Text(t) => out.push((t.clone(), body)),
            SpanKind::Emphasis(t) => out.push((t.clone(), Run { italic: true, ..body })),
            SpanKind::Strong(t) => out.push((t.clone(), Run { bold: true, ..body })),
            SpanKind::StrongEmphasis(t) => {
                out.push((t.clone(), Run { bold: true, italic: true, ..body }))
            }
            SpanKind::Code(t) => {
                out.push((t.clone(), Run { mono: true, pill: true, color: st.code, ..body }))
            }
            SpanKind::Link { text, .. } => {
                out.push((text.clone(), Run { color: st.link, ..body }))
            }
            // Images are lifted out and rendered as their own blocks; ignore here.
            SpanKind::Image { .. } => {}
            SpanKind::Strikethrough(t) => out.push((t.clone(), Run { color: st.chrome, ..body })),
            SpanKind::FootnoteRef { number, .. } => out.push((
                crate::parser::superscript(*number),
                Run { color: st.link, ..body },
            )),
            SpanKind::SoftBreak => out.push((" ".to_string(), body)),
            SpanKind::HardBreak => out.push(("\n".to_string(), body)),
        }
    }
    if out.is_empty() {
        out.push((String::new(), body));
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// Painter
// ─────────────────────────────────────────────────────────────────────────

/// A shaped block ready to paint, plus its layout metadata.
struct Block {
    buffer: Buffer,
    #[allow(dead_code)] // reserved for baseline-alignment refinements
    line_height: f32,
    height: u32,
    /// Left edge of the text, relative to the content column start.
    indent: u32,
    space_before: u32,
    /// Optional decorations drawn relative to the block's painted box.
    decorations: Vec<Decoration>,
}

/// A primitive drawn directly onto the page (bars, rules, bullets).
enum Decoration {
    /// Filled rectangle (x, y, w, h) relative to the block origin, in device px.
    Rect { x: i32, y: i32, w: u32, h: u32, color: Rgb },
    /// Pre-rendered bitmap blitted at (x, y) relative to the block origin.
    /// Used for tables, which can't fit the single-buffer block model.
    Image { x: i32, y: i32, img: RgbaImage },
}

/// Owns the font system + glyph cache. Build once; reuse across renders.
pub struct Painter {
    font_system: FontSystem,
    swash: SwashCache,
    /// True while only the core fonts (Georgia/Menlo) are loaded and the full
    /// system database is still pending (lazy-loaded after first paint).
    core_only: bool,
    /// Directory of the source markdown file, used to resolve relative image
    /// paths. `None` disables local image decoding (placeholders only).
    base_dir: Option<std::path::PathBuf>,
}

/// Build a [`FontSystem`] containing only the reader's core fonts — Georgia
/// (body) and Menlo (code) — so startup skips the ~120 ms system-font scan.
///
/// Returns `None` unless *both* families resolve, so the caller falls back to a
/// full scan on any other platform, or if a system file has moved: the worst
/// case is today's proven behavior, never a wrong or missing font.
fn fast_core_font_system() -> Option<FontSystem> {
    use cosmic_text::fontdb;

    // Only the families the reader renders. Everything else (non-Latin, emoji)
    // is covered by the full system DB, lazy-loaded after first paint.
    #[cfg(target_os = "macos")]
    const CORE_FONT_FILES: &[&str] = &[
        "/System/Library/Fonts/Supplemental/Georgia.ttf",
        "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
        "/System/Library/Fonts/Supplemental/Georgia Italic.ttf",
        "/System/Library/Fonts/Supplemental/Georgia Bold Italic.ttf",
        "/System/Library/Fonts/Menlo.ttc",
    ];
    #[cfg(not(target_os = "macos"))]
    const CORE_FONT_FILES: &[&str] = &[];

    if CORE_FONT_FILES.is_empty() {
        return None; // unsupported platform → full scan
    }

    let mut db = fontdb::Database::new();
    for path in CORE_FONT_FILES {
        db.load_font_file(path).ok()?; // missing/unreadable → full scan
    }

    // Require both families to actually be present before trusting the db.
    let has_family =
        |name: &str| db.faces().any(|f| f.families.iter().any(|(fam, _)| fam.as_str() == name));
    if !has_family("Georgia") || !has_family("Menlo") {
        return None;
    }

    db.set_serif_family("Georgia");
    db.set_sans_serif_family("Georgia");
    db.set_monospace_family("Menlo");

    Some(FontSystem::new_with_locale_and_db(detect_locale(), db))
}

/// Best-effort UI locale for font fallback, read from the environment. Only
/// affects CJK disambiguation (irrelevant to the Latin core fonts); the later
/// full-DB load inherits it. Defaults to `en-US`.
fn detect_locale() -> String {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        let Ok(val) = std::env::var(key) else { continue };
        let loc = val.split('.').next().unwrap_or(&val);
        if !loc.is_empty() && loc != "C" && loc != "POSIX" {
            return loc.replace('_', "-");
        }
    }
    "en-US".to_string()
}

/// A block placed at an absolute vertical position in the document.
struct Placed {
    buffer: Buffer,
    /// Absolute top of the block content, in page pixels.
    y_top: u32,
    height: u32,
    indent: u32,
    decorations: Vec<Decoration>,
}

/// A laid-out document. Shaping happens **incrementally** (one top-level
/// element at a time, top to bottom) so first paint only pays for the visible
/// window; the rest is shaped lazily/in the background. Rasterization likewise
/// happens on demand for a vertical window, so memory and encode cost stay
/// bounded regardless of document length.
pub struct RichDoc {
    pub page_w: u32,
    /// Exact once [`RichDoc::fully_shaped`]; until then an estimate (refined as
    /// shaping progresses) so scroll math has a stable target without paying
    /// the full shaping cost up front.
    pub total_h: u32,
    bg: Rgb,
    ink: Rgb,
    margin_px: u32,
    /// Blocks shaped + positioned so far (document top downward).
    blocks: Vec<Placed>,
    // ── Incremental shaping state ──
    /// Parsed top-level elements, shaped on demand one at a time.
    elements: Vec<Element>,
    style: DocStyle,
    /// Index of the next top-level element to shape.
    next_el: usize,
    /// Running bottom (page px) of the last positioned block — where the next
    /// block starts, before its `space_before`. Includes the top pad.
    shaped_h: u32,
    /// True once every element has been shaped; `total_h` is then exact.
    fully_shaped: bool,
}

/// The result of resolving a selection against the laid-out document: the
/// highlight rectangles to paint and the rendered text to copy. All rectangle
/// coordinates are in page **device pixels** (the same space the blocks are
/// laid out in), `(x, y, w, h)`.
pub struct SelectionRender {
    pub rects: Vec<(f32, f32, f32, f32)>,
    pub text: String,
}

impl RichDoc {
    /// Whether every element has been shaped (so `total_h` is exact).
    pub fn fully_shaped(&self) -> bool {
        self.fully_shaped
    }

    /// Resolve a selection between two page-pixel points (`anchor`, `head`) into
    /// highlight rectangles + the rendered text between them. Points are in page
    /// device pixels. Returns `None` if either endpoint doesn't land on shaped
    /// text. The two points may be given in any order.
    pub fn select(&self, anchor: (f32, f32), head: (f32, f32)) -> Option<SelectionRender> {
        let a = self.resolve(anchor)?;
        let b = self.resolve(head)?;
        // Order by (block, line, index) so start <= end.
        let (start, end) = if (a.0, a.1.line, a.1.index) <= (b.0, b.1.line, b.1.index) {
            (a, b)
        } else {
            (b, a)
        };

        let mut rects = Vec::new();
        let mut text = String::new();
        for bi in start.0..=end.0 {
            let block = &self.blocks[bi];
            let ox = (self.margin_px + block.indent) as f32;
            let last = block_end_cursor(&block.buffer);
            let cs = if bi == start.0 { start.1 } else { Cursor::new(0, 0) };
            let ce = if bi == end.0 { end.1 } else { last };

            for run in block.buffer.layout_runs() {
                for (hx, hw) in run.highlight(cs, ce) {
                    if hw > 0.0 {
                        rects.push((ox + hx, block.y_top as f32 + run.line_top, hw, run.line_height));
                    }
                }
            }

            if bi != start.0 {
                text.push('\n');
            }
            text.push_str(&block_text(&block.buffer, cs, ce));
        }

        Some(SelectionRender { rects, text })
    }

    /// Map a page-pixel point to the block it falls in and the text cursor
    /// there. Clamps vertically to the nearest block so a point in inter-block
    /// whitespace still resolves.
    fn resolve(&self, (x, y): (f32, f32)) -> Option<(usize, Cursor)> {
        if self.blocks.is_empty() {
            return None;
        }
        let bi = self
            .blocks
            .iter()
            .position(|b| y >= b.y_top as f32 && y < (b.y_top + b.height) as f32)
            .or_else(|| {
                // Below all shaped blocks → last; above the first → first.
                self.blocks
                    .iter()
                    .rposition(|b| (b.y_top as f32) <= y)
                    .or(Some(0))
            })?;
        let block = &self.blocks[bi];
        let ox = (self.margin_px + block.indent) as f32;
        let local_x = x - ox;
        let local_y = (y - block.y_top as f32).clamp(0.0, block.height.saturating_sub(1) as f32);
        let cursor = block.buffer.hit(local_x.max(0.0), local_y)?;
        Some((bi, cursor))
    }
}

/// A cursor at the very end of a buffer's text (last line, past last byte).
fn block_end_cursor(buffer: &Buffer) -> Cursor {
    let line = buffer.lines.len().saturating_sub(1);
    let index = buffer.lines.last().map(|l| l.text().len()).unwrap_or(0);
    Cursor::new(line, index)
}

/// Extract the text of one buffer between two cursors (`cs <= ce`), joining
/// logical lines with `\n`. Byte indices come from `Buffer::hit`, so they are
/// valid char boundaries; `get` guards against any out-of-range surprise.
fn block_text(buffer: &Buffer, cs: Cursor, ce: Cursor) -> String {
    let lines = &buffer.lines;
    if cs.line == ce.line {
        return lines
            .get(cs.line)
            .and_then(|l| l.text().get(cs.index..ce.index))
            .unwrap_or("")
            .to_string();
    }
    let mut s = String::new();
    if let Some(l) = lines.get(cs.line) {
        s.push_str(l.text().get(cs.index..).unwrap_or(""));
    }
    for i in (cs.line + 1)..ce.line {
        s.push('\n');
        if let Some(l) = lines.get(i) {
            s.push_str(l.text());
        }
    }
    s.push('\n');
    if let Some(l) = lines.get(ce.line) {
        s.push_str(l.text().get(..ce.index).unwrap_or(""));
    }
    s
}

impl Painter {
    pub fn new() -> Self {
        // Fast path: load only the two fonts the reader actually uses (Georgia
        // body + Menlo code) so first paint isn't blocked on a full system-font
        // scan (~120 ms). Falls back to the proven full scan on any platform or
        // missing-file mismatch — worst case is exactly today's behavior, never
        // a wrong font.
        match fast_core_font_system() {
            Some(font_system) => Self {
                font_system,
                swash: SwashCache::new(),
                core_only: true,
                base_dir: None,
            },
            None => Self {
                font_system: FontSystem::new(),
                swash: SwashCache::new(),
                core_only: false,
                base_dir: None,
            },
        }
    }

    /// Set the directory used to resolve relative image paths.
    pub fn set_base_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.base_dir = dir;
    }

    /// Whether only the core fonts are loaded (full system DB still pending).
    pub fn is_core_only(&self) -> bool {
        self.core_only
    }

    /// Load the full system font database if we started core-only. Idempotent;
    /// a no-op if we already did a full scan. Called once after first paint so
    /// later / non-Latin content gets full glyph coverage without blocking the
    /// initial render. Existing shaped content keeps its (correct) core fonts —
    /// only added fallbacks change, so nothing already on screen reflows.
    pub fn ensure_full_fonts(&mut self) {
        if self.core_only {
            self.font_system.db_mut().load_system_fonts();
            self.core_only = false;
        }
    }

    /// Begin a document: record the parsed elements + style but shape nothing
    /// yet. Cheap — actual shaping happens lazily via [`ensure_shaped_to`] /
    /// [`ensure_fully_shaped`], so first paint need only shape the top window.
    ///
    /// [`ensure_shaped_to`]: Painter::ensure_shaped_to
    /// [`ensure_fully_shaped`]: Painter::ensure_fully_shaped
    pub fn begin(&self, elements: Vec<Element>, st: &DocStyle) -> RichDoc {
        RichDoc {
            page_w: st.content_px + st.margin_px * 2,
            // Minimal until the first element is shaped; refined as we go.
            total_h: (st.pad_px * 2).max(1),
            bg: st.bg,
            ink: st.ink,
            margin_px: st.margin_px,
            blocks: Vec::new(),
            elements,
            style: st.clone(),
            next_el: 0,
            shaped_h: st.pad_px,
            fully_shaped: false,
        }
    }

    /// Lay out (shape + position) the WHOLE document. Convenience for callers
    /// that need the full page immediately (e.g. PNG export). Output is
    /// identical to shaping incrementally — top-level elements are independent.
    pub fn layout(&mut self, elements: &[Element], st: &DocStyle) -> RichDoc {
        let mut doc = self.begin(elements.to_vec(), st);
        self.ensure_fully_shaped(&mut doc);
        doc
    }

    /// Shape (and position) elements until the document is shaped at least down
    /// to page-pixel `y_target`, or fully shaped. Cheap no-op once already past
    /// `y_target`.
    pub fn ensure_shaped_to(&mut self, doc: &mut RichDoc, y_target: u32) {
        while !doc.fully_shaped && doc.shaped_h < y_target {
            self.shape_one(doc);
        }
    }

    /// Shape (and position) every remaining element. After this, `total_h` is
    /// exact.
    pub fn ensure_fully_shaped(&mut self, doc: &mut RichDoc) {
        while !doc.fully_shaped {
            self.shape_one(doc);
        }
    }

    /// Shape up to `max_elements` more elements, then return — a background
    /// trickle that stays responsive to higher-priority work between chunks.
    pub fn shape_step(&mut self, doc: &mut RichDoc, max_elements: usize) {
        for _ in 0..max_elements {
            if doc.fully_shaped {
                break;
            }
            self.shape_one(doc);
        }
    }

    /// Shape one top-level element, append its positioned blocks, and advance
    /// the running height. Updates `total_h` (exact when done, else estimate).
    fn shape_one(&mut self, doc: &mut RichDoc) {
        let i = doc.next_el;
        if i >= doc.elements.len() {
            doc.fully_shaped = true;
            return;
        }

        // Shape this element in isolation. Top-level elements are independent,
        // so the only cross-element rule — the first heading gets no space
        // before it — is reproduced by passing `top_level = (i == 0)`.
        let mut blocks: Vec<Block> = Vec::new();
        {
            let st = &doc.style;
            let el = &doc.elements[i];
            self.lay_out(std::slice::from_ref(el), st, st.content_px, 0, &mut blocks, i == 0);
        }

        // Position the new blocks below what's already placed.
        let pad = doc.style.pad_px;
        let mut y = doc.shaped_h;
        for b in blocks {
            y += b.space_before;
            doc.blocks.push(Placed {
                buffer: b.buffer,
                y_top: y,
                height: b.height,
                indent: b.indent,
                decorations: b.decorations,
            });
            y += b.height;
        }
        doc.shaped_h = y;
        doc.next_el += 1;

        if doc.next_el >= doc.elements.len() {
            doc.fully_shaped = true;
            doc.total_h = (doc.shaped_h + pad).max(1);
        } else {
            // Estimate the remainder from the average element height so far, so
            // the scroll-clamp/scrollbar has a stable (if approximate) target
            // until background shaping finishes.
            let shaped = doc.next_el as u32;
            let content_h = doc.shaped_h.saturating_sub(pad);
            let avg = (content_h / shaped.max(1)).max(1);
            let remaining = (doc.elements.len() - doc.next_el) as u32;
            doc.total_h = (doc.shaped_h + avg * remaining + pad).max(1);
        }
    }

    /// Rasterize the WHOLE document, downscaled so the total pixel count does
    /// not exceed `max_pixels`. Rendered in vertical tiles so peak memory stays
    /// bounded regardless of document length. Returns the bitmap and the scale
    /// factor applied (≤ 1.0).
    ///
    /// This is what the interactive reader transmits — once — so scrolling never
    /// re-encodes or re-transmits anything.
    pub fn render_scaled(&mut self, doc: &mut RichDoc, max_pixels: u64) -> (RgbaImage, f32) {
        self.ensure_fully_shaped(doc); // need exact total_h to size the target
        let total_px = doc.page_w as u64 * doc.total_h as u64;
        let q = if total_px > max_pixels {
            (max_pixels as f64 / total_px as f64).sqrt() as f32
        } else {
            1.0
        };

        let tw = ((doc.page_w as f32 * q).round() as u32).max(1);
        let th = ((doc.total_h as f32 * q).round() as u32).max(1);
        let bg = Rgba([doc.bg.0, doc.bg.1, doc.bg.2, 255]);
        let mut target = RgbaImage::from_pixel(tw, th, bg);

        const TILE: u32 = 4096; // source pixels per tile
        let mut y = 0u32;
        while y < doc.total_h {
            let h = TILE.min(doc.total_h - y);
            let src = self.render_window(doc, y, h); // page_w × h, full resolution
            let dy = (y as f32 * q).round() as u32;
            let dh = ((h as f32 * q).round() as u32).max(1);
            if (q - 1.0).abs() < f32::EPSILON {
                imageops::overlay(&mut target, &src, 0, dy as i64);
            } else {
                let resized = imageops::resize(&src, tw, dh, FilterType::Triangle);
                imageops::overlay(&mut target, &resized, 0, dy as i64);
            }
            y += h;
        }

        (target, q)
    }

    /// Rasterize the page-pixel range `[y0, y0 + win_h)` and scale it by
    /// `scale` (≤ 1.0), producing a `round(page_w·scale) × round(win_h·scale)`
    /// bitmap. Rendered in vertical tiles so peak memory stays bounded
    /// regardless of how tall the requested window is.
    ///
    /// Unlike [`render_scaled`], the scale is chosen by the caller (from the
    /// terminal width), so on-screen text size is independent of document
    /// length — long documents render at the same comfortable size as short
    /// ones, with off-screen regions rendered on demand while scrolling.
    pub fn render_window_scaled(
        &mut self,
        doc: &mut RichDoc,
        y0: u32,
        win_h: u32,
        scale: f32,
    ) -> RgbaImage {
        let win_h = win_h.max(1);
        let out_w = ((doc.page_w as f32 * scale).round() as u32).max(1);
        let out_h = ((win_h as f32 * scale).round() as u32).max(1);
        let bg = Rgba([doc.bg.0, doc.bg.1, doc.bg.2, 255]);
        let mut target = RgbaImage::from_pixel(out_w, out_h, bg);

        let unscaled = (scale - 1.0).abs() < f32::EPSILON;
        const TILE: u32 = 4096; // source pixels per tile
        let mut y = 0u32;
        while y < win_h {
            let h = TILE.min(win_h - y);
            let src = self.render_window(doc, y0 + y, h); // page_w × h, full resolution
            let dy = (y as f32 * scale).round() as u32;
            if unscaled {
                imageops::overlay(&mut target, &src, 0, dy as i64);
            } else {
                let dh = ((h as f32 * scale).round() as u32).max(1);
                let resized = imageops::resize(&src, out_w, dh, FilterType::Triangle);
                imageops::overlay(&mut target, &resized, 0, dy as i64);
            }
            y += h;
        }

        target
    }

    /// Rasterize only the page-pixel range `[y0, y0 + win_h)` into a bitmap of
    /// size `page_w × win_h`. Blocks outside the range are skipped.
    pub fn render_window(&mut self, doc: &mut RichDoc, y0: u32, win_h: u32) -> RgbaImage {
        let win_h = win_h.max(1);
        self.ensure_shaped_to(doc, y0 + win_h); // shape any not-yet-shaped rows
        let page_w = doc.page_w;
        let bg = Rgba([doc.bg.0, doc.bg.1, doc.bg.2, 255]);
        let mut img = RgbaImage::from_pixel(page_w, win_h, bg);
        let y1 = y0 + win_h;
        let ink = Color::rgb(doc.ink.0, doc.ink.1, doc.ink.2);
        let pill_bg = doc.style.code_bg;

        for p in &mut doc.blocks {
            // Skip blocks entirely outside the window.
            if p.y_top + p.height <= y0 || p.y_top >= y1 {
                continue;
            }
            let ox = doc.margin_px as i32 + p.indent as i32;
            let oy = p.y_top as i32 - y0 as i32;

            for d in &p.decorations {
                match d {
                    Decoration::Rect { x, y: dy, w, h, color } => {
                        fill_rect(&mut img, ox + x, oy + dy, *w, *h, *color);
                    }
                    Decoration::Image { x, y: dy, img: src } => {
                        blit_image(&mut img, src, ox + x, oy + dy);
                    }
                }
            }

            // Inline-code chips: fill a subtle rect behind each contiguous run
            // of glyphs tagged with PILL_META, before painting the glyphs.
            {
                let m = p.buffer.metrics();
                let pad_x = m.font_size * 0.22;
                let pill_h = (m.font_size * 1.32).round() as u32;
                let mut rects: Vec<(i32, i32, u32, u32)> = Vec::new();
                for run in p.buffer.layout_runs() {
                    let top = run.line_top + (m.line_height - pill_h as f32) / 2.0;
                    let mut span: Option<(f32, f32)> = None;
                    let flush = |s: Option<(f32, f32)>, rects: &mut Vec<(i32, i32, u32, u32)>| {
                        if let Some((x0, x1)) = s {
                            let rx = (ox as f32 + x0 - pad_x).round() as i32;
                            let rw = (x1 - x0 + pad_x * 2.0).round().max(1.0) as u32;
                            let ry = oy + top.round() as i32;
                            rects.push((rx, ry, rw, pill_h));
                        }
                    };
                    for g in run.glyphs.iter() {
                        if g.metadata == PILL_META {
                            let (gx0, gx1) = (g.x, g.x + g.w);
                            span = Some(match span {
                                Some((s, _)) => (s, gx1),
                                None => (gx0, gx1),
                            });
                        } else {
                            flush(span, &mut rects);
                            span = None;
                        }
                    }
                    flush(span, &mut rects);
                }
                for (rx, ry, rw, rh) in rects {
                    fill_rect(&mut img, rx, ry, rw, rh, pill_bg);
                }
            }

            let (fs, sw) = (&mut self.font_system, &mut self.swash);
            p.buffer.draw(fs, sw, ink, |gx, gy, gw, gh, color| {
                let a = color.a();
                if a == 0 {
                    return;
                }
                let rgb = (color.r(), color.g(), color.b());
                for ddy in 0..gh as i32 {
                    for ddx in 0..gw as i32 {
                        let px = ox + gx + ddx;
                        let py = oy + gy + ddy;
                        if px < 0 || py < 0 || px as u32 >= page_w || py as u32 >= win_h {
                            continue;
                        }
                        blend_pixel(&mut img, px as u32, py as u32, rgb, a);
                    }
                }
            });
        }

        img
    }

    /// Recursively lay out elements into shaped blocks.
    /// `width` is the available text width; `base_indent` shifts the column.
    fn lay_out(
        &mut self,
        elements: &[Element],
        st: &DocStyle,
        width: u32,
        base_indent: u32,
        out: &mut Vec<Block>,
        top_level: bool,
    ) {
        for el in elements {
            match el {
                Element::Heading { level, spans, .. } => {
                    let lvl = (*level).min(6);
                    let scale = DocStyle::heading_scale(lvl);
                    let size = st.base_px * scale;
                    let lh = size * 1.2;
                    let color = st.h[(lvl - 1) as usize];
                    let runs = flatten_spans(spans, st, color);
                    // H1: uppercase for editorial punch + an accent bar.
                    let (runs, indent, decos) = if lvl == 1 {
                        let upper: Vec<(String, Run)> = runs
                            .into_iter()
                            .map(|(t, r)| (t.to_uppercase(), Run { bold: true, ..r }))
                            .collect();
                        let bar_w = (st.base_px * 0.28).max(4.0) as u32;
                        let bar_gap = (st.base_px * 0.5) as u32;
                        let deco = Decoration::Rect {
                            x: 0,
                            y: (lh * 0.12) as i32,
                            w: bar_w,
                            h: (lh * 0.78) as u32,
                            color: st.accent,
                        };
                        (upper, bar_w + bar_gap, vec![deco])
                    } else {
                        let bold = lvl <= 5;
                        let runs = runs
                            .into_iter()
                            .map(|(t, r)| (t, Run { bold, ..r }))
                            .collect();
                        (runs, 0, Vec::new())
                    };
                    let (buffer, height) =
                        self.shape(&runs, st, Metrics::new(size, lh), width - indent, Align::Left);
                    out.push(Block {
                        buffer,
                        line_height: lh,
                        height,
                        indent: base_indent + indent,
                        space_before: if top_level && out.is_empty() {
                            0
                        } else {
                            st.heading_space_before(lvl)
                        },
                        decorations: decos,
                    });
                }

                Element::Paragraph { spans, .. } => {
                    let lh = st.base_px * 1.55;
                    let has_image = spans.iter().any(|s| matches!(s.kind, SpanKind::Image { .. }));
                    if !has_image {
                        let runs = flatten_spans(spans, st, st.ink);
                        let (buffer, height) =
                            self.shape(&runs, st, Metrics::new(st.base_px, lh), width, Align::Left);
                        out.push(Block {
                            buffer,
                            line_height: lh,
                            height,
                            indent: base_indent,
                            space_before: st.para_space(),
                            decorations: Vec::new(),
                        });
                    } else {
                        // Lift images out as their own blocks; shape the text
                        // segments around them as separate paragraphs.
                        let mut seg: Vec<Span> = Vec::new();
                        let flush_seg = |me: &mut Self, seg: &mut Vec<Span>, out: &mut Vec<Block>| {
                            if seg.is_empty() {
                                return;
                            }
                            let runs = flatten_spans(seg, st, st.ink);
                            let (buffer, height) =
                                me.shape(&runs, st, Metrics::new(st.base_px, lh), width, Align::Left);
                            out.push(Block {
                                buffer,
                                line_height: lh,
                                height,
                                indent: base_indent,
                                space_before: st.para_space(),
                                decorations: Vec::new(),
                            });
                            seg.clear();
                        };
                        for span in spans {
                            if let SpanKind::Image { url, alt } = &span.kind {
                                flush_seg(self, &mut seg, out);
                                let (img, height) = self.image_block(url, alt, st, width);
                                let (buffer, _) = self.shape(
                                    &[(String::new(), Run { bold: false, italic: false, mono: false, pill: false, color: st.ink })],
                                    st,
                                    Metrics::new(st.base_px, st.base_px),
                                    width,
                                    Align::Left,
                                );
                                out.push(Block {
                                    buffer,
                                    line_height: st.base_px,
                                    height,
                                    indent: base_indent,
                                    space_before: st.para_space(),
                                    decorations: vec![Decoration::Image { x: 0, y: 0, img }],
                                });
                            } else {
                                seg.push(span.clone());
                            }
                        }
                        flush_seg(self, &mut seg, out);
                    }
                }

                Element::List { ordered, start, items, .. } => {
                    let marker_w = (st.base_px * 1.6) as u32;
                    for (i, item) in items.iter().enumerate() {
                        let runs = flatten_spans(&item.spans, st, st.ink);
                        let lh = st.base_px * 1.55;
                        let (buffer, height) = self.shape(
                            &runs,
                            st,
                            Metrics::new(st.base_px, lh),
                            width.saturating_sub(marker_w),
                            Align::Left,
                        );
                        let marker_space = if i == 0 { st.para_space() } else { (st.base_px * 0.3) as u32 };
                        if let Some(checked) = item.task {
                            // Task-list checkbox glyph drawn as a small bitmap,
                            // vertically centered on the first text line.
                            let box_sz = (st.base_px * 0.9) as u32;
                            let cb = make_checkbox(checked, box_sz, st);
                            let y_off = ((lh - box_sz as f32) / 2.0).max(0.0) as i32;
                            let (mbuf, _) = self.shape(
                                &[(String::new(), Run { bold: false, italic: false, mono: false, pill: false, color: st.accent })],
                                st,
                                Metrics::new(st.base_px, lh),
                                marker_w,
                                Align::Left,
                            );
                            out.push(Block {
                                buffer: mbuf,
                                line_height: lh,
                                height: 0,
                                indent: base_indent
                                    + marker_w.saturating_sub(box_sz + (st.base_px * 0.35) as u32),
                                space_before: marker_space,
                                decorations: vec![Decoration::Image { x: 0, y: y_off, img: cb }],
                            });
                        } else {
                            // Marker as its own little shaped buffer, accent-colored.
                            let marker_text = if *ordered {
                                format!("{}.", start.unwrap_or(1) + i as u64)
                            } else {
                                "•".to_string()
                            };
                            let mrun = vec![(
                                marker_text,
                                Run { bold: false, italic: false, mono: false, pill: false, color: st.accent },
                            )];
                            let (mbuf, _) =
                                self.shape(&mrun, st, Metrics::new(st.base_px, lh), marker_w, Align::Left);
                            out.push(Block {
                                buffer: mbuf,
                                line_height: lh,
                                height: 0, // marker overlays the item; no own height
                                indent: base_indent + marker_w.saturating_sub((st.base_px * 1.1) as u32),
                                space_before: marker_space,
                                decorations: Vec::new(),
                            });
                        }
                        out.push(Block {
                            buffer,
                            line_height: lh,
                            height,
                            indent: base_indent + marker_w,
                            space_before: 0,
                            decorations: Vec::new(),
                        });
                        if !item.blocks.is_empty() {
                            self.lay_out(
                                &item.blocks,
                                st,
                                width.saturating_sub(marker_w),
                                base_indent + marker_w,
                                out,
                                false,
                            );
                        }
                    }
                }

                Element::BlockQuote { elements, .. } => {
                    let bar_w = (st.base_px * 0.18).max(3.0) as u32;
                    let gap = (st.base_px * 0.7) as u32;
                    let inner_indent = base_indent + bar_w + gap;
                    let before = out.len();
                    self.lay_out(elements, st, width.saturating_sub(bar_w + gap), inner_indent, out, false);
                    // Italicize the quote and add a vertical accent bar spanning it.
                    let mut span_h: u32 = 0;
                    for b in &mut out[before..] {
                        span_h += b.space_before + b.height;
                    }
                    if let Some(first) = out.get_mut(before) {
                        first.space_before = st.para_space();
                        first.decorations.push(Decoration::Rect {
                            x: -(gap as i32),
                            y: 0,
                            w: bar_w,
                            h: span_h.max(first.height),
                            color: st.accent,
                        });
                    }
                }

                Element::CodeBlock { code, .. } => {
                    let lh = st.base_px * 1.45;
                    let size = st.base_px * 0.92;
                    let runs = vec![(
                        code.trim_end_matches('\n').to_string(),
                        Run { bold: false, italic: false, mono: true, pill: false, color: st.ink },
                    )];
                    let indent = (st.base_px * 0.8) as u32;
                    let (buffer, height) = self.shape(
                        &runs,
                        st,
                        Metrics::new(size, lh),
                        width.saturating_sub(indent),
                        Align::Left,
                    );
                    let pad = (st.base_px * 0.5) as u32;
                    let deco = Decoration::Rect {
                        x: -(indent as i32) / 2,
                        y: -(pad as i32),
                        w: width,
                        h: height + pad * 2,
                        color: st.code_bg, // faint code panel (darker than bg)
                    };
                    out.push(Block {
                        buffer,
                        line_height: lh,
                        height: height + pad * 2,
                        indent: base_indent + indent,
                        space_before: st.para_space() + pad,
                        decorations: vec![deco],
                    });
                }

                Element::HorizontalRule { .. } => {
                    // A short centered rule with breathing room.
                    let lh = st.base_px;
                    let (buffer, _) = self.shape(&[(String::new(), Run { bold: false, italic: false, mono: false, pill: false, color: st.ink })], st, Metrics::new(st.base_px, lh), width, Align::Left);
                    let rule_w = width / 4;
                    let rule_x = (width / 2 - rule_w / 2) as i32;
                    out.push(Block {
                        buffer,
                        line_height: lh,
                        height: (st.base_px * 1.2) as u32,
                        indent: base_indent,
                        space_before: (st.base_px * 1.2) as u32,
                        decorations: vec![Decoration::Rect {
                            x: rule_x,
                            y: (st.base_px * 0.5) as i32,
                            w: rule_w,
                            h: (st.base_px * 0.06).max(2.0) as u32,
                            color: st.chrome,
                        }],
                    });
                }

                Element::Table { headers, rows, aligns } => {
                    if let Some((img, height)) = self.paint_table(headers, rows, aligns, st, width) {
                        // The table is pre-rasterized into its own bitmap (the
                        // block model is vertical-flow only); carry it as an
                        // Image decoration over an empty placeholder buffer.
                        let (buffer, _) = self.shape(
                            &[(String::new(), Run { bold: false, italic: false, mono: false, pill: false, color: st.ink })],
                            st,
                            Metrics::new(st.base_px, st.base_px),
                            width,
                            Align::Left,
                        );
                        out.push(Block {
                            buffer,
                            line_height: st.base_px,
                            height,
                            indent: base_indent,
                            space_before: st.para_space(),
                            decorations: vec![Decoration::Image { x: 0, y: 0, img }],
                        });
                    }
                }
                Element::Chart { chart } => {
                    // Rasterized to its own bitmap (like tables) and carried as
                    // an Image decoration over an empty placeholder buffer.
                    let img = super::chart_render::render_chart(chart, width, st);
                    let height = img.height();
                    let (buffer, _) = self.shape(
                        &[(String::new(), Run { bold: false, italic: false, mono: false, pill: false, color: st.ink })],
                        st,
                        Metrics::new(st.base_px, st.base_px),
                        width,
                        Align::Left,
                    );
                    out.push(Block {
                        buffer,
                        line_height: st.base_px,
                        height,
                        indent: base_indent,
                        space_before: st.para_space(),
                        decorations: vec![Decoration::Image { x: 0, y: 0, img }],
                    });
                }
            }
        }
    }

    /// Shape a set of styled runs into a Buffer and measure its pixel height.
    fn shape(
        &mut self,
        runs: &[(String, Run)],
        st: &DocStyle,
        metrics: Metrics,
        width: u32,
        align: Align,
    ) -> (Buffer, u32) {
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(width as f32), None);

        let default = Attrs::new().family(Family::Name(st.body_font));
        let spans: Vec<(&str, Attrs)> =
            runs.iter().map(|(t, r)| (t.as_str(), r.attrs(st))).collect();
        buffer.set_rich_text(spans, &default, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut bottom = 0.0_f32;
        for run in buffer.layout_runs() {
            let b = run.line_top + metrics.line_height;
            if b > bottom {
                bottom = b;
            }
        }
        (buffer, bottom.ceil() as u32)
    }

    /// Measure the pixel width of a single unwrapped run of text.
    fn measure_width(&mut self, text: &str, bold: bool, st: &DocStyle, size: f32) -> f32 {
        let run = Run { bold, italic: false, mono: false, pill: false, color: st.ink };
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, size * 1.4));
        buffer.set_size(Some(1.0e6), None);
        let default = Attrs::new().family(Family::Name(st.body_font));
        buffer.set_rich_text([(text, run.attrs(st))], &default, Shaping::Advanced, Some(Align::Left));
        buffer.shape_until_scroll(&mut self.font_system, false);
        // Use glyph extents (right edge of the last glyph) rather than `line_w`,
        // whose value depends on the set buffer width and isn't the text advance.
        buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter().map(|g| g.x + g.w))
            .fold(0.0_f32, f32::max)
    }

    /// Rasterize a shaped buffer onto `img` at (ox, oy), alpha-blending glyphs.
    fn blit_buffer(&mut self, img: &mut RgbaImage, buffer: &mut Buffer, ox: i32, oy: i32, ink: Color) {
        let (iw, ih) = (img.width(), img.height());
        let (fs, sw) = (&mut self.font_system, &mut self.swash);
        buffer.draw(fs, sw, ink, |gx, gy, gw, gh, color| {
            let a = color.a();
            if a == 0 {
                return;
            }
            let rgb = (color.r(), color.g(), color.b());
            for ddy in 0..gh as i32 {
                for ddx in 0..gw as i32 {
                    let px = ox + gx + ddx;
                    let py = oy + gy + ddy;
                    if px < 0 || py < 0 || px as u32 >= iw || py as u32 >= ih {
                        continue;
                    }
                    blend_pixel(img, px as u32, py as u32, rgb, a);
                }
            }
        });
    }

    /// Rasterize a GFM table into its own bitmap: column widths fitted to the
    /// available measure, per-column alignment, a bold tinted header, and
    /// editorial horizontal rules (no vertical lines). Returns `None` for an
    /// empty table.
    fn paint_table(
        &mut self,
        headers: &[String],
        rows: &[Vec<String>],
        aligns: &[CellAlign],
        st: &DocStyle,
        width: u32,
    ) -> Option<(RgbaImage, u32)> {
        let ncol = headers
            .len()
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if ncol == 0 {
            return None;
        }

        let fs = st.base_px * 0.95;
        let lh = fs * 1.4;
        let pad_x = (st.base_px * 0.55).round();
        let pad_y = (st.base_px * 0.40).round();
        let cell = |row: &[String], c: usize| -> String {
            row.get(c).cloned().unwrap_or_default()
        };

        // 1. Natural column widths from measured single-line content.
        let mut natural = vec![0.0_f32; ncol];
        for (c, nat) in natural.iter_mut().enumerate() {
            let mut w = self.measure_width(&cell(headers, c), true, st, fs);
            for row in rows {
                w = w.max(self.measure_width(&cell(row, c), false, st, fs));
            }
            // Small slack so a cell that measured to fit doesn't wrap on a
            // sub-pixel rounding boundary during the final shaping pass.
            *nat = w + pad_x * 2.0 + fs * 0.3;
        }
        let total_natural: f32 = natural.iter().sum();

        // 2. Keep natural widths if they fit the measure; else scale down to fit
        //    (cells then wrap) with a per-column minimum.
        let avail = width as f32;
        let col_w: Vec<u32> = if total_natural <= avail {
            natural.iter().map(|w| (w.round() as u32).max(1)).collect()
        } else {
            let factor = avail / total_natural;
            let min_w = (fs * 3.0).round();
            natural
                .iter()
                .map(|w| ((w * factor).max(min_w).round() as u32).max(1))
                .collect()
        };
        let table_w: u32 = col_w.iter().sum();
        if table_w == 0 {
            return None;
        }

        let to_align = |a: CellAlign| match a {
            CellAlign::Left => Align::Left,
            CellAlign::Center => Align::Center,
            CellAlign::Right => Align::Right,
        };

        // 3. Shape every cell at its column content width; row height = tallest.
        let mut logical: Vec<(bool, Vec<String>)> = Vec::with_capacity(rows.len() + 1);
        logical.push((true, headers.to_vec()));
        for row in rows {
            logical.push((false, row.clone()));
        }
        let mut shaped: Vec<Vec<Buffer>> = Vec::with_capacity(logical.len());
        let mut row_h: Vec<u32> = Vec::with_capacity(logical.len());
        for (is_header, cells) in &logical {
            let mut srow = Vec::with_capacity(ncol);
            let mut h = 0u32;
            for (c, cw) in col_w.iter().enumerate() {
                let content_w = cw.saturating_sub(pad_x as u32 * 2).max(1);
                let align = to_align(aligns.get(c).copied().unwrap_or(CellAlign::Left));
                let run = Run { bold: *is_header, italic: false, mono: false, pill: false, color: st.ink };
                let (buf, bh) =
                    self.shape(&[(cell(cells, c), run)], st, Metrics::new(fs, lh), content_w, align);
                h = h.max(bh);
                srow.push(buf);
            }
            row_h.push(h + pad_y as u32 * 2);
            shaped.push(srow);
        }

        // Column x offsets.
        let mut col_x = vec![0u32; ncol];
        let mut acc = 0u32;
        for (c, cw) in col_w.iter().enumerate() {
            col_x[c] = acc;
            acc += cw;
        }
        // Row y offsets + total height.
        let mut row_y = vec![0u32; row_h.len()];
        let mut yacc = 0u32;
        for (r, h) in row_h.iter().enumerate() {
            row_y[r] = yacc;
            yacc += h;
        }
        let total_h = yacc.max(1);

        // 4. Compose. Subtle header tint, then cells, then editorial rules.
        let mut img = RgbaImage::from_pixel(table_w, total_h, Rgba([st.bg.0, st.bg.1, st.bg.2, 255]));
        fill_rect(&mut img, 0, 0, table_w, row_h[0], st.code_bg);

        let ink = Color::rgb(st.ink.0, st.ink.1, st.ink.2);
        for (r, srow) in shaped.iter_mut().enumerate() {
            for (c, buf) in srow.iter_mut().enumerate() {
                let x = col_x[c] as i32 + pad_x as i32;
                let y = row_y[r] as i32 + pad_y as i32;
                self.blit_buffer(&mut img, buf, x, y, ink);
            }
        }

        let rule_h = (st.base_px * 0.05).max(2.0) as u32;
        let strong = (170u8, 170u8, 170u8);
        let light = (224u8, 224u8, 224u8);
        // Top, header separator, and bottom: stronger. Inter-row: light.
        fill_rect(&mut img, 0, 0, table_w, rule_h, strong);
        fill_rect(&mut img, 0, row_h[0] as i32 - rule_h as i32, table_w, rule_h, strong);
        for r in 2..row_y.len() {
            fill_rect(&mut img, 0, row_y[r] as i32, table_w, rule_h.min(2).max(1), light);
        }
        fill_rect(&mut img, 0, total_h as i32 - rule_h as i32, table_w, rule_h, strong);

        Some((img, total_h))
    }

    /// Decode a local image file referenced by `url`, resolving relative paths
    /// against the configured base directory. Returns `None` for remote URLs,
    /// missing files, or decode failures.
    fn load_local_image(&self, url: &str) -> Option<RgbaImage> {
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:") {
            return None;
        }
        let raw = url.strip_prefix("file://").unwrap_or(url);
        let p = std::path::Path::new(raw);
        let path = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.as_ref()?.join(p)
        };
        image::open(&path).ok().map(|i| i.to_rgba8())
    }

    /// Render an image reference into a bitmap: the decoded image scaled to the
    /// measure when it's a readable local file, otherwise a framed placeholder
    /// captioned with the alt text.
    fn image_block(&mut self, url: &str, alt: &str, st: &DocStyle, width: u32) -> (RgbaImage, u32) {
        if let Some(decoded) = self.load_local_image(url) {
            let (iw, ih) = decoded.dimensions();
            if iw > 0 && ih > 0 {
                // Fit to the measure; never upscale beyond natural size.
                let scale = if iw > width { width as f32 / iw as f32 } else { 1.0 };
                let tw = ((iw as f32 * scale).round() as u32).max(1);
                let th = ((ih as f32 * scale).round() as u32).max(1);
                let scaled = if (scale - 1.0).abs() < f32::EPSILON {
                    decoded
                } else {
                    imageops::resize(&decoded, tw, th, FilterType::Triangle)
                };
                return (scaled, th);
            }
        }
        self.image_placeholder(alt, st, width)
    }

    /// A framed, tinted placeholder box captioned with the alt text — shown for
    /// remote or unreadable images.
    fn image_placeholder(&mut self, alt: &str, st: &DocStyle, width: u32) -> (RgbaImage, u32) {
        let pad = (st.base_px * 0.8) as u32;
        let caption = if alt.trim().is_empty() { "image" } else { alt };
        let cap_run = Run { bold: false, italic: true, mono: false, pill: false, color: st.code };
        let (mut cbuf, ch) = self.shape(
            &[(caption.to_string(), cap_run)],
            st,
            Metrics::new(st.base_px * 0.95, st.base_px * 1.3),
            width.saturating_sub(pad * 2).max(1),
            Align::Center,
        );
        let h = ch + pad * 2;
        let mut img =
            RgbaImage::from_pixel(width, h, Rgba([st.code_bg.0, st.code_bg.1, st.code_bg.2, 255]));
        // Border.
        let b = (st.base_px * 0.05).max(2.0) as u32;
        fill_rect(&mut img, 0, 0, width, b, st.chrome);
        fill_rect(&mut img, 0, (h - b) as i32, width, b, st.chrome);
        fill_rect(&mut img, 0, 0, b, h, st.chrome);
        fill_rect(&mut img, (width - b) as i32, 0, b, h, st.chrome);
        // Caption, vertically centered.
        let ink = Color::rgb(st.code.0, st.code.1, st.code.2);
        self.blit_buffer(&mut img, &mut cbuf, pad as i32, ((h - ch) / 2) as i32, ink);
        (img, h)
    }
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Pixel helpers
// ─────────────────────────────────────────────────────────────────────────

/// Alpha-blend `rgb` over the existing pixel using coverage `a` (0-255).
fn blend_pixel(img: &mut RgbaImage, x: u32, y: u32, rgb: Rgb, a: u8) {
    let dst = img.get_pixel_mut(x, y);
    let af = a as f32 / 255.0;
    let blend = |s: u8, d: u8| -> u8 {
        (s as f32 * af + d as f32 * (1.0 - af)).round().clamp(0.0, 255.0) as u8
    };
    dst[0] = blend(rgb.0, dst[0]);
    dst[1] = blend(rgb.1, dst[1]);
    dst[2] = blend(rgb.2, dst[2]);
    dst[3] = 255;
}

/// Blit `src` onto `dst` at (x, y), alpha-blending each pixel and clipping to
/// the destination bounds.
fn blit_image(dst: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32) {
    let (dw, dh) = (dst.width(), dst.height());
    for (sx, sy, px) in src.enumerate_pixels() {
        let cx = x + sx as i32;
        let cy = y + sy as i32;
        if cx < 0 || cy < 0 || cx as u32 >= dw || cy as u32 >= dh {
            continue;
        }
        blend_pixel(dst, cx as u32, cy as u32, (px[0], px[1], px[2]), px[3]);
    }
}

/// Draw a thick line by stamping square dabs along it. Crude but crisp at 2x.
fn draw_thick_line(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, thick: f32, color: Rgb) {
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let steps = (len.ceil() as i32 * 2).max(1);
    let t = thick.max(1.0);
    let r = (t / 2.0) as i32;
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let x = (x0 + (x1 - x0) * f).round() as i32 - r;
        let y = (y0 + (y1 - y0) * f).round() as i32 - r;
        fill_rect(img, x, y, t as u32, t as u32, color);
    }
}

/// Render a GFM task-list checkbox into a transparent `size`×`size` bitmap:
/// an outlined square when unchecked, a filled square with a check when done.
fn make_checkbox(checked: bool, size: u32, st: &DocStyle) -> RgbaImage {
    let s = size.max(6);
    let mut img = RgbaImage::from_pixel(s, s, Rgba([0, 0, 0, 0]));
    let stroke = ((s as f32 * 0.10).round() as u32).max(2);
    if checked {
        // Solid accent square with a white check for a clear "done" signal.
        fill_rect(&mut img, 0, 0, s, s, st.accent);
        let sf = s as f32;
        let t = (sf * 0.13).max(2.0);
        draw_thick_line(&mut img, sf * 0.24, sf * 0.54, sf * 0.43, sf * 0.72, t, st.bg);
        draw_thick_line(&mut img, sf * 0.43, sf * 0.72, sf * 0.78, sf * 0.30, t, st.bg);
    } else {
        // Hollow outlined square.
        fill_rect(&mut img, 0, 0, s, stroke, st.accent);
        fill_rect(&mut img, 0, (s - stroke) as i32, s, stroke, st.accent);
        fill_rect(&mut img, 0, 0, stroke, s, st.accent);
        fill_rect(&mut img, (s - stroke) as i32, 0, stroke, s, st.accent);
    }
    img
}

/// Fill an opaque rectangle, clipped to the image bounds.
fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, color: Rgb) {
    let (iw, ih) = (img.width(), img.height());
    let px = Rgba([color.0, color.1, color.2, 255]);
    for dy in 0..h as i32 {
        for dx in 0..w as i32 {
            let cx = x + dx;
            let cy = y + dy;
            if cx < 0 || cy < 0 || cx as u32 >= iw || cy as u32 >= ih {
                continue;
            }
            img.put_pixel(cx as u32, cy as u32, px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Representative long-form markdown — the mix of headings, prose, lists,
    /// quotes and code the rich reader actually renders.
    fn sample_markdown(sections: usize) -> String {
        let mut s = String::from(
            "# The Rich Reader\n\nAn editorial markdown reading experience with real typography.\n\n",
        );
        for i in 0..sections {
            s.push_str(&format!("## Section {i}: on typography\n\n"));
            s.push_str(
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
                 tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
                 quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
                 consequat. Duis aute irure dolor in reprehenderit in voluptate velit.\n\n",
            );
            s.push_str("- First point worth making here\n- Second supporting detail to consider\n- Third and final observation\n\n");
            s.push_str("> A pulled quote that adds emphasis and a different visual texture.\n\n");
            s.push_str("```\nfn main() {\n    println!(\"hello, world\");\n}\n```\n\n");
        }
        s
    }

    /// Render a representative band of roughly `px_budget` pixels and return its
    /// dimensions plus raw RGBA bytes.
    fn render_band(px_budget: u32) -> (u32, u32, Vec<u8>) {
        let md = sample_markdown(40);
        let style = DocStyle::light();
        let (mut painter, mut doc) = crate::rich::lay_out_document(&md, &style, None);
        let page_w = doc.page_w;
        let win_h = (px_budget / page_w).max(1);
        let img = painter.render_window(&mut doc, 0, win_h);
        (page_w, win_h, img.into_raw())
    }

    /// Fast premise guard: a representative rendered band's RGBA must compress
    /// dramatically. This is the whole reason the vendored kitty `o=z` transmit
    /// is cheap — if rendered content ever stops being highly compressible (e.g.
    /// a photographic background), this fails and the scroll-perf assumption is
    /// no longer valid.
    #[test]
    fn band_rgba_is_highly_compressible() {
        let (_w, _h, raw) = render_band(2_000_000);
        let c = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        assert!(
            c.len() * 5 < raw.len(),
            "expected >5x compression of a rendered band, raw={} compressed={}",
            raw.len(),
            c.len(),
        );
    }

    /// Perf exploration (ignored by default). Run for realistic numbers with:
    ///   cargo test --release -- --ignored --nocapture band_compression_table
    #[test]
    #[ignore = "perf exploration; prints a table — run with --release --nocapture"]
    fn band_compression_table() {
        let (page_w, win_h, raw) = render_band(8_000_000);
        println!(
            "\nband {page_w}x{win_h} = {} px   raw RGBA = {:.1} MB",
            page_w * win_h,
            raw.len() as f64 / 1e6,
        );
        for level in [1u8, 2, 4, 6, 9] {
            let t = Instant::now();
            let c = miniz_oxide::deflate::compress_to_vec_zlib(&raw, level);
            let ms = t.elapsed().as_millis();
            println!(
                "  zlib L{level}: {:.2} MB   {:.0}x smaller   {ms} ms",
                c.len() as f64 / 1e6,
                raw.len() as f64 / c.len() as f64,
            );
        }
    }

    /// Per-band phase breakdown (ignored by default). Decomposes the worker's
    /// per-band cost into layout / rasterize / RGBA-clone / zlib so we can see
    /// which phase to optimize — reproducibly and offline, unlike hand-scrolling.
    /// Run with:
    ///   cargo test --release -- --ignored --nocapture band_phase_breakdown
    #[test]
    #[ignore = "perf exploration; prints a phase table — run with --release --nocapture"]
    fn band_phase_breakdown() {
        let md = sample_markdown(40);
        let style = DocStyle::light();

        // First-paint decomposition. The live reader pays parse + FontSystem +
        // begin up front, then shapes lazily; `ensure_fully_shaped` is the
        // worst case (whole doc), shown separately from the one-time font scan.
        let t = Instant::now();
        let elements = crate::parser::parse(&md);
        let parse_ms = t.elapsed().as_millis();

        let t = Instant::now();
        let mut painter = Painter::new();
        let core_fonts_ms = t.elapsed().as_millis();

        // Comparison only: the full system-font scan the fast path replaces.
        let t = Instant::now();
        let _full = cosmic_text::FontSystem::new();
        let full_scan_ms = t.elapsed().as_millis();

        let t = Instant::now();
        let mut doc = painter.begin(elements, &style);
        let begin_ms = t.elapsed().as_millis();

        let t = Instant::now();
        painter.ensure_fully_shaped(&mut doc);
        let shape_ms = t.elapsed().as_millis();

        let page_w = doc.page_w;
        let win_h = (8_000_000u32 / page_w).max(1);

        let t = Instant::now();
        let img = painter.render_window(&mut doc, 0, win_h);
        let raster_ms = t.elapsed().as_millis();
        let (bw, bh) = (img.width(), img.height());

        // The live transmit path wraps this RgbaImage in a DynamicImage and calls
        // to_rgba8(), which re-clones the whole buffer. Measure that redundant copy.
        let dynimg = image::DynamicImage::ImageRgba8(img);
        let t = Instant::now();
        let rgba = dynimg.to_rgba8();
        let clone_ms = t.elapsed().as_millis();
        let raw = rgba.as_raw();

        println!(
            "\nband {bw}x{bh} = {} px   raw RGBA = {:.1} MB",
            bw * bh,
            raw.len() as f64 / 1e6,
        );
        println!("  parse                            : {parse_ms} ms   (1x, startup)");
        println!("  Painter::new (core fonts, fast)  : {core_fonts_ms} ms   (1x, startup)");
        println!("  [vs] FontSystem::new full scan   : {full_scan_ms} ms   (replaced)");
        println!("  begin (lazy setup)               : {begin_ms} ms   (1x, startup)");
        println!("  shape WHOLE doc (worst case)     : {shape_ms} ms   (1x, lazy/trickled live)");
        println!("  raster (render_window)           : {raster_ms} ms   (per band)");
        println!("  rgba   (redundant to_rgba8)      : {clone_ms} ms   (per band)");
        for level in [1u8, 2, 4] {
            let t = Instant::now();
            let c = miniz_oxide::deflate::compress_to_vec_zlib(raw, level);
            let ms = t.elapsed().as_millis();
            println!(
                "  zlib L{level}                          : {ms} ms  ->  {:.2} MB ({:.0}x)",
                c.len() as f64 / 1e6,
                raw.len() as f64 / c.len() as f64,
            );
        }

        // Candidate: transmit RGB (f=24) instead of RGBA. Bands are opaque, so
        // zlib scans 24 MB instead of 32 MB. Measure conversion + compress vs L2.
        let t = Instant::now();
        let rgb = dynimg.to_rgb8();
        let rgb_conv_ms = t.elapsed().as_millis();
        let rgb_raw = rgb.as_raw();
        let t = Instant::now();
        let c2 = miniz_oxide::deflate::compress_to_vec_zlib(rgb_raw, 2);
        let rgb_zlib_ms = t.elapsed().as_millis();
        println!(
            "  RGB f=24 cand : to_rgb8 {rgb_conv_ms} ms + zlib L2 {rgb_zlib_ms} ms -> {:.2} MB  ({:.1} MB raw)",
            c2.len() as f64 / 1e6,
            rgb_raw.len() as f64 / 1e6,
        );
    }
}
