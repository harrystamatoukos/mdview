//! Read-only "rich" reader: displays the document inline via a terminal
//! graphics protocol and scrolls it smoothly.
//!
//! ## Why a background worker
//!
//! Two costs scale with document length and would otherwise stutter the UI:
//!
//! * **Shaping** (cosmic-text layout) — hundreds of ms for a long document.
//! * **Rasterize + base64-encode** of each multi-megabyte band.
//!
//! Both are moved off the UI thread onto a dedicated **render worker** that
//! owns the `Painter` (and therefore the `FontSystem`, which never crosses a
//! thread boundary). The UI thread only sends a *target* (which band it wants)
//! and displays finished `SlicedProtocol`s as they arrive — it never shapes or
//! encodes, so input and scrolling stay responsive regardless of length.
//!
//! The worker shapes **lazily** (top window first → fast first paint), then
//! trickle-shapes the rest in the background, and **prefetches** the bands on
//! either side of the current one so crossing a band edge is already paid for.
//! Within a band, scrolling only moves a cell offset (`SlicedImage`) — nothing
//! is re-encoded or re-transmitted.

use anyhow::{anyhow, Result};
use std::io::stdout;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use image::DynamicImage;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Rect, Size},
    style::{Color, Style},
    widgets::{Block, Paragraph},
    Terminal,
};
use ratatui_image::{
    picker::Picker,
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};

use super::{DocStyle, Painter, RichDoc};

/// A point in page device pixels.
type PxPoint = (f32, f32);
/// A coalesced selection request: `(request id, anchor, head)`.
type PendingSelect = (u64, PxPoint, PxPoint);

const EVENT_POLL_MS: u64 = 100;
/// Faster poll while waiting for the worker to deliver the band under the
/// viewport, so the new frame appears the moment it lands.
const WAIT_POLL_MS: u64 = 16;
const MAX_EVENTS_PER_FRAME: usize = 64;
/// Pixel budget for one transmitted *band*. Raw RGBA, so bytes ≈ pixels × 4.
/// The document is shown one band at a time; bands are sized to this budget so
/// each transmission stays bounded no matter how long the document is.
const MAX_PIXELS: u64 = 8_000_000;
/// Lower bound on the width-fit scale, so an extremely narrow terminal can't
/// blow up the per-band render. Below this the page is centered/clipped instead.
const MIN_SCALE: f32 = 0.25;
/// How many elements the worker shapes per background trickle step before
/// re-checking for higher-priority band requests.
const SHAPE_CHUNK: usize = 64;
/// Distinct bands the worker remembers having produced (to skip re-rendering).
/// Kept smaller than the UI cache so the UI always still holds anything the
/// worker considers "already produced" — no stale-blank, no needless re-encode.
const PRODUCED_CAP: usize = 3;
/// Decoded bands the UI keeps around (LRU by arrival). ≥ current + 2 neighbors.
const CACHE_CAP: usize = 6;

enum RichEvent {
    Quit,
    Changed,
    Ignored,
}

// ── Perf instrumentation (enabled with MDVIEW_PERF=1) ──────────────────────
// Writes timings to $TMPDIR/mdview-perf.log. The TUI owns stdout, so we can't
// print to the screen; a log file keeps the measurements out of the way.

fn perf_enabled() -> bool {
    std::env::var_os("MDVIEW_PERF").is_some()
}

fn perf_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mdview-perf.log")
}

fn perf_log(msg: &str) {
    if !perf_enabled() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(perf_path())
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// Launch the rich reader for the given markdown source.
pub fn run(markdown: &str, base_dir: Option<std::path::PathBuf>) -> Result<()> {
    if perf_enabled() {
        let _ = std::fs::write(perf_path(), b"=== mdview perf ===\n");
        eprintln!("[mdview] perf logging to {}", perf_path().display());
    }

    enable_raw_mode()?;

    let picker = Picker::from_query_stdio().map_err(|e| {
        let _ = disable_raw_mode();
        anyhow!(
            "could not initialize terminal graphics ({e}).\n\
             The default reader needs a graphics-capable terminal \
             (Ghostty, Kitty, iTerm2, or WezTerm). Try `mdview --tui <file>`."
        )
    })?;

    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &picker, markdown, base_dir);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    result
}

// ─────────────────────────────────────────────────────────────────────────
// UI ⇄ worker protocol
// ─────────────────────────────────────────────────────────────────────────

/// What the UI currently wants on screen. Sent whenever it changes; the worker
/// always works toward the latest one. All fields are `Copy`/cheap.
#[derive(Clone, Copy, PartialEq)]
struct Target {
    /// Band top, in document cell-rows at `scale` (an aligned slot).
    row0: u32,
    /// Band height in cell-rows (stable; independent of document length).
    rows: u32,
    /// `f32::to_bits` of the width-fit scale this target is for.
    scale_bits: u32,
    cell_w: u32,
    cell_h: u32,
    /// Distance between adjacent band slots (rows minus overlap).
    stride: u32,
    /// Largest valid `row0` (last band is pinned to the document bottom).
    max_row0: u32,
}

enum Request {
    SetTarget(Target),
    /// Resolve a selection between two page-pixel points. `id` lets the UI drop
    /// stale results from an in-flight drag.
    Select {
        id: u64,
        anchor: (f32, f32),
        head: (f32, f32),
    },
    Quit,
}

enum Response {
    /// A finished, encoded band ready to display.
    Band {
        row0: u32,
        rows: u32,
        cols: u16,
        scale_bits: u32,
        proto: SlicedProtocol,
        total_h: u32,
        fully_shaped: bool,
    },
    /// Background-shaping progress: the document's height estimate grew (or
    /// became exact). Lets the UI refine its scroll bounds.
    Progress { total_h: u32, fully_shaped: bool },
    /// Resolved selection: highlight rects (page device px) + text to copy.
    /// `id` echoes the request so the UI can ignore stale drag results.
    Selection {
        id: u64,
        rects: Vec<(f32, f32, f32, f32)>,
        text: String,
    },
}

/// A decoded band held by the UI. Scrolling within `[row0, row0 + rows)` only
/// moves a cell offset; leaving that range switches to another cached band (or
/// waits for the worker to deliver it).
struct CachedBand {
    proto: SlicedProtocol,
    row0: u32,
    rows: u32,
    cols: u16,
    scale_bits: u32,
}

impl CachedBand {
    /// Whether this band fully covers the viewport `[top, top + view_h)`.
    fn covers(&self, scale_bits: u32, top: u32, view_h: u32) -> bool {
        self.scale_bits == scale_bits && self.row0 <= top && self.row0 + self.rows >= top + view_h
    }
}

/// UI-side selection state for mouse drag-to-copy. Endpoints and rects live in
/// page **device pixels** (resolved by the worker); the UI converts screen
/// cells to that space on press/drag and converts rects back for the overlay.
#[derive(Default)]
struct Selection {
    /// Drag anchor in page device px (fixed at mouse-down).
    anchor: (f32, f32),
    /// Latest highlight rectangles from the worker (page device px).
    rects: Vec<(f32, f32, f32, f32)>,
    /// A drag is currently in progress.
    dragging: bool,
    /// Id of the most recent `Select` request (monotonic; lets us drop stale
    /// results that land out of order during a fast drag).
    req_id: u64,
    /// When set, copy the text of the `Selection` response carrying this id
    /// (set on mouse-up so the copy reflects the final drag position).
    copy_id: Option<u64>,
}

impl Selection {
    /// Whether there's anything to draw.
    fn is_active(&self) -> bool {
        !self.rects.is_empty()
    }

    /// Clear any active selection. Returns whether something was cleared (so the
    /// caller can mark the frame dirty).
    fn clear(&mut self) -> bool {
        let had = self.is_active() || self.dragging;
        self.rects.clear();
        self.dragging = false;
        self.copy_id = None;
        had
    }
}

/// Convert a screen cell `(col, row)` to a page **device-pixel** point, given
/// the current scale `s`, scroll position, and horizontal centering offset.
/// Inverse of the draw mapping: page is laid out at device px and shown at
/// scale `s`, centered at `x_off` cells, with the viewport top at `scroll_rows`.
fn cell_to_page_px(
    col: u16,
    row: u16,
    x_off: u16,
    scroll_rows: u32,
    cell_w: u32,
    cell_h: u32,
    s: f32,
) -> (f32, f32) {
    let icol = col.saturating_sub(x_off) as f32;
    let px = icol * cell_w as f32 / s;
    let py = (scroll_rows as f32 + row as f32) * cell_h as f32 / s;
    (px, py)
}

// ─────────────────────────────────────────────────────────────────────────
// Render worker
// ─────────────────────────────────────────────────────────────────────────

/// Owns the `Painter` (and `FontSystem`) and `RichDoc`. Serves the UI's latest
/// target band first, then prefetches neighbors, then trickle-shapes the rest
/// in the background, then blocks until the next request.
fn render_worker(
    markdown: String,
    style: DocStyle,
    picker: Picker,
    base_dir: Option<std::path::PathBuf>,
    req_rx: Receiver<Request>,
    resp_tx: Sender<Response>,
) {
    let t_parse = std::time::Instant::now();
    let elements = crate::parser::parse(&markdown);
    let parse_ms = t_parse.elapsed().as_millis();
    let n_elements = elements.len();
    let t_fs = std::time::Instant::now();
    let mut painter = Painter::new();
    painter.set_base_dir(base_dir);
    let fs_ms = t_fs.elapsed().as_millis();
    let t_begin = std::time::Instant::now();
    let mut doc = painter.begin(elements, &style);
    let begin_ms = t_begin.elapsed().as_millis();
    perf_log(&format!(
        "worker init: {} ms [parse {parse_ms} + FontSystem {fs_ms} + begin {begin_ms}] \
         ({n_elements} elements, page_w={})",
        parse_ms + fs_ms + begin_ms,
        doc.page_w
    ));

    let mut target: Option<Target> = None;
    // Recency-ordered keys of bands already produced (front = oldest).
    let mut produced: Vec<(u32, u32)> = Vec::with_capacity(PRODUCED_CAP);

    // Latest selection request, coalesced across a drag's event stream.
    let mut pending_select: Option<PendingSelect> = None;

    'main: loop {
        // 1. Drain all queued requests; keep only the latest target / selection.
        loop {
            match req_rx.try_recv() {
                Ok(Request::Quit) => return,
                Ok(Request::SetTarget(t)) => target = Some(t),
                Ok(Request::Select { id, anchor, head }) => {
                    pending_select = Some((id, anchor, head));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // 1b. Selection takes priority over band work so a drag feels live.
        if let Some((id, anchor, head)) = pending_select.take() {
            let (rects, text) = match doc.select(anchor, head) {
                Some(s) => (s.rects, s.text),
                None => (Vec::new(), String::new()),
            };
            let _ = resp_tx.send(Response::Selection { id, rects, text });
            continue 'main;
        }

        if let Some(t) = target {
            // 2. Serve the target band first (highest priority).
            let key = (t.row0, t.scale_bits);
            if !produced.contains(&key) {
                match render_band(&mut painter, &mut doc, &picker, &t, t.row0) {
                    Some(resp) => {
                        let _ = resp_tx.send(resp);
                    }
                    None => {
                        // Target fell past the (now exact) document end; report
                        // height so the UI re-clamps and picks a valid target.
                        let _ = resp_tx.send(Response::Progress {
                            total_h: doc.total_h,
                            fully_shaped: doc.fully_shaped(),
                        });
                    }
                }
                remember(&mut produced, key);
                continue 'main; // re-check for a newer target before doing more
            }

            // 3. Target satisfied → prefetch the neighboring slots.
            for nrow0 in neighbor_row0s(&t) {
                let nkey = (nrow0, t.scale_bits);
                if !produced.contains(&nkey) {
                    if let Some(resp) = render_band(&mut painter, &mut doc, &picker, &t, nrow0) {
                        let _ = resp_tx.send(resp);
                    }
                    remember(&mut produced, nkey);
                    continue 'main;
                }
            }
        }

        // 3b. One-time: the first band(s) are on screen using the fast core
        // fonts. Now load the full system database (non-Latin / emoji coverage
        // for the rest of the document). Done here — after the target and its
        // neighbors are served, with the worker caught up — so it never delays
        // first paint or an active scroll. No-op unless we started core-only.
        if painter.is_core_only() && !produced.is_empty() {
            painter.ensure_full_fonts();
            let _ = resp_tx.send(Response::Progress {
                total_h: doc.total_h,
                fully_shaped: doc.fully_shaped(),
            });
            continue 'main;
        }

        // 4. Nothing urgent → trickle-shape the rest in the background.
        if !doc.fully_shaped() {
            painter.shape_step(&mut doc, SHAPE_CHUNK);
            let _ = resp_tx.send(Response::Progress {
                total_h: doc.total_h,
                fully_shaped: doc.fully_shaped(),
            });
            continue 'main;
        }

        // 5. Idle (shaped, target served, prefetched) → block for next request.
        match req_rx.recv() {
            Ok(Request::SetTarget(t)) => target = Some(t),
            Ok(Request::Select { id, anchor, head }) => {
                pending_select = Some((id, anchor, head));
            }
            Ok(Request::Quit) | Err(_) => return,
        }
    }
}

/// Render + encode one band at cell-row `row0` for the given target geometry.
/// Returns `None` if `row0` lies past the (fully-shaped) document end.
fn render_band(
    painter: &mut Painter,
    doc: &mut RichDoc,
    picker: &Picker,
    t: &Target,
    row0: u32,
) -> Option<Response> {
    let scale = f32::from_bits(t.scale_bits);
    let cell_h = t.cell_h as u64;
    let cell_w = t.cell_w;

    // Map the band's cell range back to a document-pixel window.
    let y0_doc = ((row0 as u64 * cell_h) as f64 / scale as f64).round() as u32;
    let y_bottom = (((row0 + t.rows) as u64 * cell_h) as f64 / scale as f64).round() as u32;
    // Ensure shaping has reached the band bottom (may make total_h exact).
    painter.ensure_shaped_to(doc, y_bottom);

    if y0_doc >= doc.total_h {
        return None; // past the end
    }
    let win_h_doc = ((t.rows as u64 * cell_h) as f64 / scale as f64).round() as u32;
    let win_h_doc = win_h_doc.min(doc.total_h.saturating_sub(y0_doc)).max(1);

    let t_raster = std::time::Instant::now();
    let img = painter.render_window_scaled(doc, y0_doc, win_h_doc, scale);
    let raster_ms = t_raster.elapsed().as_millis();
    let (bw, bh) = (img.width(), img.height());
    let cols = bw.div_ceil(cell_w).max(1) as u16;
    let rows = bh.div_ceil(t.cell_h).max(1);
    // `encode` is SlicedProtocol::new, which slices the band and runs the
    // vendored kitty transmit (to_rgba8 + zlib + base64) — broken down further
    // by the "  transmit:" sub-line that transmit_virtual logs.
    let t_enc = std::time::Instant::now();
    let proto = SlicedProtocol::new(
        picker,
        DynamicImage::ImageRgba8(img),
        Some(Size::new(cols, rows as u16)),
    )
    .ok()?;
    let enc_ms = t_enc.elapsed().as_millis();
    perf_log(&format!(
        "band render: {} ms [raster {raster_ms} + encode {enc_ms}] \
         (s={scale:.3}, row0={row0}, img={bw}x{bh}, cols={cols}, rows={rows})",
        raster_ms + enc_ms
    ));

    Some(Response::Band {
        row0,
        rows,
        cols,
        scale_bits: t.scale_bits,
        proto,
        total_h: doc.total_h,
        fully_shaped: doc.fully_shaped(),
    })
}

/// The neighboring band slots to prefetch (down first — the common direction).
fn neighbor_row0s(t: &Target) -> Vec<u32> {
    let mut v = Vec::with_capacity(2);
    let down = (t.row0 + t.stride).min(t.max_row0);
    if down != t.row0 {
        v.push(down);
    }
    if t.row0 >= t.stride {
        let up = (t.row0 - t.stride).min(t.max_row0);
        if up != t.row0 && !v.contains(&up) {
            v.push(up);
        }
    }
    v
}

/// Record a produced band key, keeping only the most recent `PRODUCED_CAP`.
fn remember(produced: &mut Vec<(u32, u32)>, key: (u32, u32)) {
    produced.retain(|&k| k != key);
    produced.push(key);
    if produced.len() > PRODUCED_CAP {
        produced.remove(0);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// UI loop
// ─────────────────────────────────────────────────────────────────────────

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    picker: &Picker,
    markdown: &str,
    base_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    let style = DocStyle::light();
    let bg = Color::Rgb(style.bg.0, style.bg.1, style.bg.2);
    // page_w is deterministic from the style — no need to wait on the worker.
    let page_w = style.content_px + style.margin_px * 2;

    let fs = picker.font_size();
    let cell_w = fs.width.max(1) as u32;
    let cell_h = fs.height.max(1) as u32;

    // Spawn the render worker (owns the Painter / FontSystem).
    let (req_tx, req_rx) = mpsc::channel::<Request>();
    let (resp_tx, resp_rx) = mpsc::channel::<Response>();
    let worker = {
        let markdown = markdown.to_string();
        let style = style.clone();
        let picker = picker.clone();
        std::thread::spawn(move || render_worker(markdown, style, picker, base_dir, req_rx, resp_tx))
    };

    let result = ui_loop(
        terminal, bg, page_w, cell_w, cell_h, &req_tx, &resp_rx,
    );

    // Tell the worker to exit (best-effort) and let it wind down.
    let _ = req_tx.send(Request::Quit);
    drop(req_tx);
    let _ = worker.join();
    result
}

#[allow(clippy::too_many_arguments)]
fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    bg: Color,
    page_w: u32,
    cell_w: u32,
    cell_h: u32,
    req_tx: &Sender<Request>,
    resp_rx: &Receiver<Response>,
) -> Result<()> {
    let mut scroll_rows: u32 = 0;
    let mut doc_total_h: u32 = 0; // unknown until the worker reports
    let mut fully_shaped = false;
    let mut cache: Vec<CachedBand> = Vec::new();
    let mut last_target: Option<Target> = None;
    let mut frame: u64 = 0;
    let mut dirty = true;
    let mut selection = Selection::default();
    let mut overlay = super::overlay::Overlay::default();

    loop {
        let term = terminal.size()?;
        let (term_w, term_h) = (term.width, term.height);
        let view_h = term_h.saturating_sub(1).max(1); // reserve status row
        if term_w == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // Width-fit scale: largest (never magnified past full resolution) while
        // the page fits the terminal width. Independent of document length.
        let term_w_px = term_w as u32 * cell_w;
        let s = (term_w_px as f32 / page_w as f32).clamp(MIN_SCALE, 1.0);
        let scale_bits = s.to_bits();

        // Document height as scrollable cell-rows at this scale (0 until known).
        let doc_rows = if doc_total_h > 0 {
            ((doc_total_h as f32 * s).round() as u32)
                .div_ceil(cell_h)
                .max(1)
        } else {
            view_h as u32
        };
        let max_scroll = doc_rows.saturating_sub(view_h as u32);
        if scroll_rows > max_scroll {
            scroll_rows = max_scroll;
        }

        // Band geometry — stable (independent of document length) so a band's
        // identity doesn't change as the height estimate is refined.
        let sw = ((page_w as f32 * s).round() as u32).max(1);
        let band_rows = ((MAX_PIXELS / (sw as u64 * cell_h as u64)) as u32).max(view_h as u32 + 4);
        let max_row0 = doc_rows.saturating_sub(band_rows);
        // Adjacent bands overlap by at least a viewport, so any viewport fits
        // entirely inside exactly one slot.
        let stride = band_rows.saturating_sub(view_h as u32 + 2).max(1);
        let band_index = scroll_rows / stride;
        let target_row0 = (band_index * stride).min(max_row0);

        let target = Target {
            row0: target_row0,
            rows: band_rows,
            scale_bits,
            cell_w,
            cell_h,
            stride,
            max_row0,
        };

        // 1. Drain worker responses.
        loop {
            match resp_rx.try_recv() {
                Ok(Response::Progress { total_h, fully_shaped: fs }) => {
                    doc_total_h = total_h;
                    fully_shaped = fs;
                    dirty = true;
                }
                Ok(Response::Band {
                    row0,
                    rows,
                    cols,
                    scale_bits,
                    proto,
                    total_h,
                    fully_shaped: fs,
                }) => {
                    doc_total_h = total_h;
                    fully_shaped = fs;
                    insert_band(
                        &mut cache,
                        CachedBand { proto, row0, rows, cols, scale_bits },
                    );
                    dirty = true;
                }
                Ok(Response::Selection { id, rects, text }) => {
                    // Only the latest request's result matters; drop stale ones
                    // from a fast drag so the highlight doesn't flicker backward.
                    if id == selection.req_id {
                        selection.rects = rects;
                        if selection.copy_id == Some(id) {
                            super::clipboard::copy(&text);
                            selection.copy_id = None;
                        }
                        dirty = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // 2. Tell the worker what we want, when it changes.
        if last_target != Some(target) {
            let _ = req_tx.send(Request::SetTarget(target));
            last_target = Some(target);
            dirty = true;
        }

        // 3. Choose the band to display: one that fully covers the viewport at
        //    this scale, else the nearest same-scale band (partial), else none.
        let view_top = scroll_rows;
        let covering = cache
            .iter()
            .find(|b| b.covers(scale_bits, view_top, view_h as u32));
        let chosen = covering.or_else(|| {
            cache
                .iter()
                .filter(|b| b.scale_bits == scale_bits)
                .min_by_key(|b| b.row0.abs_diff(view_top))
        });
        let covered = covering.is_some();

        // Horizontal centering offset (cells) of the page on screen, mirroring
        // the draw path, so screen↔page-pixel mapping for selection (and the
        // overlay placement) lines up with the band.
        let cols_disp = sw.div_ceil(cell_w) as u16;
        let x_off = term_w.saturating_sub(cols_disp) / 2;

        // 4. Draw.
        if dirty {
            let pct = if max_scroll == 0 {
                100
            } else {
                (scroll_rows as f32 / max_scroll as f32 * 100.0).round() as u32
            };
            let state = if chosen.is_none() {
                " · loading"
            } else if !covered {
                " · rendering"
            } else if !fully_shaped {
                " · indexing"
            } else {
                ""
            };
            let status = format!(
                " mdview · reading · {pct}%{state}    ↑/↓ j/k scroll · space page · g/G top/bottom · q quit "
            );
            let status_rect = Rect::new(0, term_h.saturating_sub(1), term_w, 1);
            let full = Rect::new(0, 0, term_w, view_h);

            let td = std::time::Instant::now();
            terminal.draw(|f| {
                f.render_widget(Block::default().style(Style::default().bg(bg)), full);
                if let Some(b) = chosen {
                    // Offset of the viewport top within the band. When a fully
                    // covering band isn't ready yet (a fast scroll outran the
                    // worker), clamp into the band's valid range so its content
                    // can't slide off-screen into a blank frame — the view
                    // "sticks" to the nearest rendered content and snaps to the
                    // exact position the instant the covering band lands. When
                    // `covered`, this is a no-op: `covers()` already guarantees
                    // the offset lies in `[0, rows - view_h]`.
                    let max_off = b.rows.saturating_sub(view_h as u32) as i64;
                    let offset = (view_top as i64 - b.row0 as i64).clamp(0, max_off) as i16;
                    let x_off = (term_w.saturating_sub(b.cols)) / 2;
                    let img_area = Rect::new(x_off, 0, b.cols.min(term_w), view_h);
                    let position = SignedPosition::from((0, -offset));
                    f.render_widget(SlicedImage::new(&b.proto, position), img_area);
                }
                f.render_widget(
                    Paragraph::new(status)
                        .style(Style::default().fg(Color::Rgb(120, 120, 120)).bg(bg)),
                    status_rect,
                );
            })?;
            frame += 1;
            perf_log(&format!(
                "draw #{frame}: {} ms (scroll_rows={scroll_rows}, covered={covered})",
                td.elapsed().as_millis()
            ));
            dirty = false;

            // Selection highlight sits above the band as a separate kitty
            // placement; refresh it whenever the frame changed (scroll moves the
            // rects; a drag changes them). Empty rects hide it.
            let mut out = stdout();
            overlay.paint(
                &mut out,
                &selection.rects,
                x_off,
                scroll_rows,
                cell_h,
                view_h,
                sw,
                s,
            );
        }

        // 5. Input. Poll faster while waiting for the covering band to arrive.
        let page = view_h.saturating_sub(2).max(1) as u32;
        let poll_ms = if covered { EVENT_POLL_MS } else { WAIT_POLL_MS };
        if event::poll(Duration::from_millis(poll_ms))? {
            for _ in 0..MAX_EVENTS_PER_FRAME {
                let ev = event::read()?;
                if handle_selection(
                    &ev, &mut selection, req_tx, x_off, scroll_rows, cell_w, cell_h, s,
                ) {
                    dirty = true;
                } else {
                    match handle_event(ev, &mut scroll_rows, max_scroll, page) {
                        RichEvent::Quit => return Ok(()),
                        RichEvent::Changed => dirty = true,
                        RichEvent::Ignored => {}
                    }
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

/// Handle left-button mouse events that drive text selection. Returns `true` if
/// the event was a selection event (and thus consumed). Shift+drag never
/// reaches us — terminals reserve it for native selection — so plain left-drag
/// is unambiguous here.
#[allow(clippy::too_many_arguments)]
fn handle_selection(
    ev: &Event,
    selection: &mut Selection,
    req_tx: &Sender<Request>,
    x_off: u16,
    scroll_rows: u32,
    cell_w: u32,
    cell_h: u32,
    s: f32,
) -> bool {
    let Event::Mouse(m) = ev else { return false };
    let point =
        || cell_to_page_px(m.column, m.row, x_off, scroll_rows, cell_w, cell_h, s);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            selection.clear();
            selection.anchor = point();
            selection.dragging = true;
            true
        }
        MouseEventKind::Drag(MouseButton::Left) if selection.dragging => {
            selection.req_id += 1;
            let _ = req_tx.send(Request::Select {
                id: selection.req_id,
                anchor: selection.anchor,
                head: point(),
            });
            true
        }
        MouseEventKind::Up(MouseButton::Left) if selection.dragging => {
            selection.dragging = false;
            // Final resolve; copy when its result returns.
            selection.req_id += 1;
            selection.copy_id = Some(selection.req_id);
            let _ = req_tx.send(Request::Select {
                id: selection.req_id,
                anchor: selection.anchor,
                head: point(),
            });
            true
        }
        _ => false,
    }
}

/// Insert a band into the UI cache (LRU by arrival; dedup by key).
fn insert_band(cache: &mut Vec<CachedBand>, band: CachedBand) {
    cache.retain(|b| !(b.row0 == band.row0 && b.scale_bits == band.scale_bits));
    cache.push(band);
    if cache.len() > CACHE_CAP {
        cache.remove(0);
    }
}

fn handle_event(input: Event, scroll: &mut u32, max_scroll: u32, page: u32) -> RichEvent {
    match input {
        Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
            KeyCode::Char('q') | KeyCode::Esc => RichEvent::Quit,
            KeyCode::Down | KeyCode::Char('j') => {
                *scroll = scroll.saturating_add(3).min(max_scroll);
                RichEvent::Changed
            }
            KeyCode::Up | KeyCode::Char('k') => {
                *scroll = scroll.saturating_sub(3);
                RichEvent::Changed
            }
            KeyCode::Char(' ') | KeyCode::PageDown => {
                *scroll = scroll.saturating_add(page).min(max_scroll);
                RichEvent::Changed
            }
            KeyCode::PageUp => {
                *scroll = scroll.saturating_sub(page);
                RichEvent::Changed
            }
            KeyCode::Char('g') | KeyCode::Home => {
                *scroll = 0;
                RichEvent::Changed
            }
            KeyCode::Char('G') | KeyCode::End => {
                *scroll = max_scroll;
                RichEvent::Changed
            }
            _ => RichEvent::Ignored,
        },
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollDown => {
                *scroll = scroll.saturating_add(2).min(max_scroll);
                RichEvent::Changed
            }
            MouseEventKind::ScrollUp => {
                *scroll = scroll.saturating_sub(2);
                RichEvent::Changed
            }
            _ => RichEvent::Ignored,
        },
        Event::Resize(_, _) => RichEvent::Changed,
        _ => RichEvent::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a moderately structured document so shaping touches headings,
    /// paragraphs, lists, code, and quotes.
    fn sample_markdown(reps: usize) -> String {
        let unit = "# Heading One\n\nA paragraph with **bold**, *italic*, and `code` spans \
that wraps across the measure to exercise line breaking.\n\n\
- first bullet\n- second bullet\n\n> a quoted line\n\n\
```\nfn main() {}\n```\n\n## Heading Two\n\nClosing paragraph.\n\n";
        unit.repeat(reps)
    }

    fn headless_picker() -> Picker {
        // No TTY in tests; halfblocks avoids the terminal query. (The internal
        // font size is irrelevant — we always pass an explicit cell Size.)
        Picker::halfblocks()
    }

    /// The worker must deliver a usable band for a target, report a height, and
    /// shut down cleanly — exercising the full channel round-trip without a
    /// terminal (the part the offline PNG path can't cover).
    #[test]
    fn worker_serves_band_and_shuts_down() {
        let md = sample_markdown(40);
        let style = DocStyle::light();
        let (cell_w, cell_h) = (8u32, 16u32);
        let scale = 0.5f32;

        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();
        let handle = {
            let style = style.clone();
            let picker = headless_picker();
            std::thread::spawn(move || render_worker(md, style, picker, None, req_rx, resp_tx))
        };

        let target = Target {
            row0: 0,
            rows: 80,
            scale_bits: scale.to_bits(),
            cell_w,
            cell_h,
            stride: 60,
            max_row0: 10_000, // generous; the band clamps to real content
        };
        req_tx.send(Request::SetTarget(target)).unwrap();

        // Progress messages may precede the band; wait until a Band arrives.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut got_band = false;
        while std::time::Instant::now() < deadline {
            match resp_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Response::Band { rows, cols, total_h, scale_bits, row0, .. }) => {
                    assert_eq!(row0, 0);
                    assert_eq!(scale_bits, scale.to_bits());
                    assert!(cols > 0, "band should have a positive cell width");
                    assert!(rows > 0, "band should have a positive cell height");
                    assert!(total_h > 0, "document height should be reported");
                    got_band = true;
                    break;
                }
                Ok(Response::Progress { total_h, .. }) => {
                    assert!(total_h > 0);
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(got_band, "worker never produced a band");

        req_tx.send(Request::Quit).unwrap();
        handle.join().expect("worker thread should exit cleanly");
    }

    /// A target past the end of the document must not hang or panic: the worker
    /// reports the (now exact) height instead of a band.
    #[test]
    fn worker_handles_target_past_end() {
        let md = sample_markdown(2); // short doc
        let style = DocStyle::light();

        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();
        let handle = {
            let style = style.clone();
            let picker = headless_picker();
            std::thread::spawn(move || render_worker(md, style, picker, None, req_rx, resp_tx))
        };

        // row0 far below any real content.
        let target = Target {
            row0: 1_000_000,
            rows: 80,
            scale_bits: 0.5f32.to_bits(),
            cell_w: 8,
            cell_h: 16,
            stride: 60,
            max_row0: 1_000_000,
        };
        req_tx.send(Request::SetTarget(target)).unwrap();

        // We should still get a height report (Progress, or a clamped Band) and
        // be able to shut down — the key property is "no hang/panic".
        let resp = resp_rx.recv_timeout(Duration::from_secs(20));
        assert!(resp.is_ok(), "worker should respond even for an out-of-range target");

        req_tx.send(Request::Quit).unwrap();
        handle.join().expect("worker thread should exit cleanly");
    }

    #[test]
    fn neighbors_are_clamped_and_distinct() {
        // Middle band: both neighbors valid and distinct.
        let t = Target { row0: 100, rows: 80, scale_bits: 0, cell_w: 8, cell_h: 16, stride: 60, max_row0: 500 };
        let mut n = neighbor_row0s(&t);
        n.sort_unstable();
        assert_eq!(n, vec![40, 160]);

        // Top band: no upward neighbor.
        let t = Target { row0: 0, rows: 80, scale_bits: 0, cell_w: 8, cell_h: 16, stride: 60, max_row0: 500 };
        assert_eq!(neighbor_row0s(&t), vec![60]);

        // Bottom band (row0 == max_row0): downward neighbor clamps away, only up.
        let t = Target { row0: 500, rows: 80, scale_bits: 0, cell_w: 8, cell_h: 16, stride: 60, max_row0: 500 };
        assert_eq!(neighbor_row0s(&t), vec![440]);
    }

    #[test]
    fn cache_is_lru_with_dedup() {
        let mk = |row0: u32| CachedBand {
            // A trivial 1x1 protocol via the headless picker.
            proto: SlicedProtocol::new(
                &headless_picker(),
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(8, 16, image::Rgba([255, 255, 255, 255]))),
                Some(Size::new(1, 1)),
            )
            .unwrap(),
            row0,
            rows: 80,
            cols: 1,
            scale_bits: 0,
        };
        let mut cache: Vec<CachedBand> = Vec::new();
        for i in 0..(CACHE_CAP as u32 + 2) {
            insert_band(&mut cache, mk(i * 10));
        }
        assert_eq!(cache.len(), CACHE_CAP, "cache is capped");
        // Oldest two evicted; newest retained.
        assert!(cache.iter().any(|b| b.row0 == (CACHE_CAP as u32 + 1) * 10));
        assert!(!cache.iter().any(|b| b.row0 == 0));

        // Re-inserting an existing key dedups (no growth, moves to newest).
        let before = cache.len();
        let keep = cache[cache.len() - 1].row0;
        insert_band(&mut cache, mk(keep));
        assert_eq!(cache.len(), before, "dedup keeps length stable");
        assert_eq!(cache.last().unwrap().row0, keep, "re-inserted band is newest");
    }
}
