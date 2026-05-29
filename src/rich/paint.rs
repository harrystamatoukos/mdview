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
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight,
};
use image::{imageops, imageops::FilterType, Rgba, RgbaImage};

use crate::parser::{Element, Span, SpanKind};

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
    color: Rgb,
}

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
        a
    }
}

/// Flatten inline spans into (text, Run) pairs, resolving emphasis/strong/
/// code/link to concrete weights, styles, and colors.
fn flatten_spans(spans: &[Span], st: &DocStyle, base_color: Rgb) -> Vec<(String, Run)> {
    let body = Run { bold: false, italic: false, mono: false, color: base_color };
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
                out.push((t.clone(), Run { mono: true, color: st.code, ..body }))
            }
            SpanKind::Link { text, .. } => {
                out.push((text.clone(), Run { color: st.link, ..body }))
            }
            SpanKind::Strikethrough(t) => out.push((t.clone(), Run { color: st.chrome, ..body })),
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
}

/// Owns the font system + glyph cache. Build once; reuse across renders.
pub struct Painter {
    font_system: FontSystem,
    swash: SwashCache,
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

impl RichDoc {
    /// Whether every element has been shaped (so `total_h` is exact).
    pub fn fully_shaped(&self) -> bool {
        self.fully_shaped
    }
}

impl Painter {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
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

        for p in &mut doc.blocks {
            // Skip blocks entirely outside the window.
            if p.y_top + p.height <= y0 || p.y_top >= y1 {
                continue;
            }
            let ox = doc.margin_px as i32 + p.indent as i32;
            let oy = p.y_top as i32 - y0 as i32;

            for d in &p.decorations {
                match *d {
                    Decoration::Rect { x, y: dy, w, h, color } => {
                        fill_rect(&mut img, ox + x, oy + dy, w, h, color);
                    }
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
                    let runs = flatten_spans(spans, st, st.ink);
                    let lh = st.base_px * 1.55;
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
                        // Marker as its own little shaped buffer, accent-colored.
                        let marker_text = if *ordered {
                            format!("{}.", start.unwrap_or(1) + i as u64)
                        } else {
                            "•".to_string()
                        };
                        let mrun = vec![(
                            marker_text,
                            Run { bold: false, italic: false, mono: false, color: st.accent },
                        )];
                        let (mbuf, _) =
                            self.shape(&mrun, st, Metrics::new(st.base_px, lh), marker_w, Align::Left);
                        out.push(Block {
                            buffer: mbuf,
                            line_height: lh,
                            height: 0, // marker overlays the item; no own height
                            indent: base_indent + marker_w.saturating_sub((st.base_px * 1.1) as u32),
                            space_before: if i == 0 { st.para_space() } else { (st.base_px * 0.3) as u32 },
                            decorations: Vec::new(),
                        });
                        out.push(Block {
                            buffer,
                            line_height: lh,
                            height,
                            indent: base_indent + marker_w,
                            space_before: 0,
                            decorations: Vec::new(),
                        });
                        if let Some(nested) = &item.nested {
                            self.lay_out(
                                std::slice::from_ref(nested.as_ref()),
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
                        Run { bold: false, italic: false, mono: true, color: st.ink },
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
                        color: (240, 240, 240), // faint code panel (darker than bg)
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
                    let (buffer, _) = self.shape(&[(String::new(), Run { bold: false, italic: false, mono: false, color: st.ink })], st, Metrics::new(st.base_px, lh), width, Align::Left);
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

                Element::Table { headers, rows, .. } => {
                    // Simple monospace rendering for now.
                    let mut text = String::new();
                    text.push_str(&headers.join("    "));
                    text.push('\n');
                    for row in rows {
                        text.push_str(&row.join("    "));
                        text.push('\n');
                    }
                    let lh = st.base_px * 1.5;
                    let runs = vec![(text, Run { bold: false, italic: false, mono: true, color: st.ink })];
                    let (buffer, height) =
                        self.shape(&runs, st, Metrics::new(st.base_px * 0.9, lh), width, Align::Left);
                    out.push(Block {
                        buffer,
                        line_height: lh,
                        height,
                        indent: base_indent,
                        space_before: st.para_space(),
                        decorations: Vec::new(),
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
        let (mut painter, mut doc) = crate::rich::lay_out_document(&md, &style);
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
}
