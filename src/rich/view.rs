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

use anyhow::{Result, anyhow};
use std::io::stdout;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::DynamicImage;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use ratatui_image::{
    picker::Picker,
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use std::path::{Path, PathBuf};

use super::{CaretMotion, DocStyle, Painter, RichDoc};
use crate::filetree::FileTree;

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
/// Default sidebar width in cells (capped to a third of the terminal).
const SIDEBAR_W: u16 = 32;
/// Minimum page columns to keep the reader usable. If the sidebar would leave
/// fewer than this, it yields for that frame (page reclaims the full width).
const MIN_CONTENT_CELLS: u16 = 24;

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

/// Launch the rich reader, browsing the markdown files under `root` with a
/// sidebar. `focus`, when given, is the file to open first (and reveal/select in
/// the tree); otherwise the first file in display order is opened.
pub fn run(root: &Path, focus: Option<&Path>) -> Result<()> {
    if perf_enabled() {
        let _ = std::fs::write(perf_path(), b"=== mdview perf ===\n");
        eprintln!("[mdview] perf logging to {}", perf_path().display());
    }

    // Build the tree. If no explicit focus, fall back to the first file so the
    // sidebar opens with something selected and revealed.
    let initial = match focus {
        Some(f) => Some(f.to_path_buf()),
        None => FileTree::build(root, None)
            .ok()
            .and_then(|t| t.first_file()),
    };
    let mut tree = FileTree::build(root, initial.as_deref())?;

    let (markdown, base_dir, title) = match initial.as_deref() {
        Some(path) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match std::fs::read_to_string(path) {
                Ok(content) => (content, path.parent().map(|p| p.to_path_buf()), name),
                Err(e) => (
                    String::new(),
                    path.parent().map(|p| p.to_path_buf()),
                    format!("⚠ cannot open {name}: {e}"),
                ),
            }
        }
        // No markdown anywhere under root: open an empty reader alongside the
        // (empty) sidebar rather than erroring.
        None => (String::new(), Some(root.to_path_buf()), String::new()),
    };

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

    let result = run_loop(
        &mut terminal,
        &picker,
        &markdown,
        base_dir,
        &mut tree,
        title,
    );

    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
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
    /// Document generation this target belongs to. Bumped on every file switch
    /// so a band rendered for a previous document can be discarded — the
    /// width-fit scale is identical across a switch, so `scale_bits` alone can't
    /// tell an in-flight old-doc band apart from a current one.
    epoch: u64,
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
    /// Switch to a different document, reusing the worker's `Painter`/`FontSystem`.
    /// `epoch` becomes the worker's current generation; responses are stamped with it.
    Load {
        epoch: u64,
        markdown: String,
        base_dir: Option<PathBuf>,
    },
    /// Move/act the keyboard caret. `id` lets the UI drop stale results; `top` is
    /// the viewport top in page px, used to place the caret on first use.
    Caret {
        id: u64,
        motion: CaretMotion,
        top: u32,
    },
    Quit,
}

enum Response {
    /// A finished, encoded band ready to display.
    Band {
        epoch: u64,
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
    Progress {
        epoch: u64,
        total_h: u32,
        fully_shaped: bool,
    },
    /// Resolved selection: highlight rects (page device px) + text to copy.
    /// `id` echoes the request so the UI can ignore stale drag results.
    Selection {
        epoch: u64,
        id: u64,
        rects: Vec<(f32, f32, f32, f32)>,
        text: String,
    },
    /// Keyboard caret state after an action: the caret bar rect (None = hidden),
    /// selection highlight rects, and copy text (only on a Copy action).
    Caret {
        epoch: u64,
        id: u64,
        caret: Option<(f32, f32, f32, f32)>,
        rects: Vec<(f32, f32, f32, f32)>,
        copy_text: Option<String>,
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

/// Re-parse `markdown` and lay it out on the *existing* `Painter`, reusing its
/// `FontSystem` (the expensive resource). Used to switch documents without
/// respawning the worker. The returned `RichDoc` replaces the previous one.
fn reload_doc(
    painter: &mut Painter,
    style: &DocStyle,
    markdown: &str,
    base_dir: Option<PathBuf>,
) -> RichDoc {
    let elements = crate::parser::parse(markdown);
    painter.set_base_dir(base_dir);
    painter.begin(elements, style)
}

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
    // Current document generation. Bumped (by the UI, echoed here) on every file
    // switch; stamped into every response so the UI can drop old-doc bands.
    let mut epoch: u64 = 0;
    // Recency-ordered keys of bands already produced (front = oldest).
    let mut produced: Vec<(u32, u32)> = Vec::with_capacity(PRODUCED_CAP);

    // Latest selection request, coalesced across a drag's event stream.
    let mut pending_select: Option<PendingSelect> = None;
    // Queued caret actions, applied in order (NOT coalesced — dropping a `v`
    // before a move, or a `y`, would corrupt the selection flow). Movement
    // flooding is bounded because we render only the final state.
    let mut pending_caret: Vec<(u64, CaretMotion, u32)> = Vec::new();

    'main: loop {
        // 1. Drain all queued requests; keep only the latest target / selection.
        loop {
            match req_rx.try_recv() {
                Ok(Request::Quit) => return,
                Ok(Request::SetTarget(t)) => target = Some(t),
                Ok(Request::Select { id, anchor, head }) => {
                    pending_select = Some((id, anchor, head));
                }
                Ok(Request::Caret { id, motion, top }) => {
                    pending_caret.push((id, motion, top));
                }
                Ok(Request::Load {
                    epoch: g,
                    markdown,
                    base_dir,
                }) => {
                    // Switch documents: re-lay out on the existing painter, then
                    // forget everything tied to the old doc.
                    epoch = g;
                    doc = reload_doc(&mut painter, &style, &markdown, base_dir);
                    produced.clear();
                    pending_select = None;
                    pending_caret.clear();
                    target = None;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // 1b. Selection / caret take priority over band work so they feel live.
        if let Some((id, anchor, head)) = pending_select.take() {
            let (rects, text) = match doc.select(anchor, head) {
                Some(s) => (s.rects, s.text),
                None => (Vec::new(), String::new()),
            };
            let _ = resp_tx.send(Response::Selection {
                epoch,
                id,
                rects,
                text,
            });
            continue 'main;
        }
        if !pending_caret.is_empty() {
            // Apply every queued action in order; render only the final state,
            // but surface any copy text produced along the way.
            let batch = std::mem::take(&mut pending_caret);
            let mut last_id = 0;
            let mut caret = None;
            let mut rects = Vec::new();
            let mut copy_text = None;
            for (id, motion, top) in batch {
                let r = painter.caret_apply(&mut doc, motion, top);
                last_id = id;
                caret = r.caret;
                rects = r.rects;
                if r.copy_text.is_some() {
                    copy_text = r.copy_text;
                }
            }
            let _ = resp_tx.send(Response::Caret {
                epoch,
                id: last_id,
                caret,
                rects,
                copy_text,
            });
            continue 'main;
        }

        if let Some(t) = target {
            // 2. Serve the target band first (highest priority).
            let key = (t.row0, t.scale_bits);
            if !produced.contains(&key) {
                match render_band(&mut painter, &mut doc, &picker, &t, t.row0, epoch) {
                    Some(resp) => {
                        let _ = resp_tx.send(resp);
                    }
                    None => {
                        // Target fell past the (now exact) document end; report
                        // height so the UI re-clamps and picks a valid target.
                        let _ = resp_tx.send(Response::Progress {
                            epoch,
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
                    if let Some(resp) =
                        render_band(&mut painter, &mut doc, &picker, &t, nrow0, epoch)
                    {
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
                epoch,
                total_h: doc.total_h,
                fully_shaped: doc.fully_shaped(),
            });
            continue 'main;
        }

        // 4. Nothing urgent → trickle-shape the rest in the background.
        if !doc.fully_shaped() {
            painter.shape_step(&mut doc, SHAPE_CHUNK);
            let _ = resp_tx.send(Response::Progress {
                epoch,
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
            Ok(Request::Caret { id, motion, top }) => {
                pending_caret.push((id, motion, top));
            }
            Ok(Request::Load {
                epoch: g,
                markdown,
                base_dir,
            }) => {
                epoch = g;
                doc = reload_doc(&mut painter, &style, &markdown, base_dir);
                produced.clear();
                pending_select = None;
                pending_caret.clear();
                target = None;
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
    epoch: u64,
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
        epoch,
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
    base_dir: Option<PathBuf>,
    tree: &mut FileTree,
    title: String,
) -> Result<()> {
    let style = DocStyle::light();
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
        std::thread::spawn(move || {
            render_worker(markdown, style, picker, base_dir, req_rx, resp_tx)
        })
    };

    let result = ui_loop(
        terminal, &style, page_w, cell_w, cell_h, &req_tx, &resp_rx, tree, title,
    );

    // Tell the worker to exit (best-effort) and let it wind down.
    let _ = req_tx.send(Request::Quit);
    drop(req_tx);
    let _ = worker.join();
    result
}

/// Which pane currently receives keyboard navigation. Mouse is always routed by
/// column, independent of this.
#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Reader,
    Sidebar,
}

/// File-search input state, orthogonal to [`Focus`]. `Typing` captures all
/// keystrokes into the query; `Results` means the sidebar shows the ranked hits
/// (navigated as the `Sidebar` focus) and remembers the query for the status row.
enum Search {
    Idle,
    Typing(String),
    Results { query: String, truncated: bool },
}

impl Search {
    fn is_typing(&self) -> bool {
        matches!(self, Search::Typing(_))
    }
    fn is_active(&self) -> bool {
        !matches!(self, Search::Idle)
    }
}

/// UI-side echo of the worker's caret: the latest caret bar + selection rects
/// (page device px) and a monotonic request id for stale-result dropping.
#[derive(Default)]
struct CaretUi {
    /// In cursor mode (opt-in): reading keys move the caret instead of scrolling.
    mode: bool,
    /// A visual selection is being extended (anchor dropped).
    selecting: bool,
    rect: Option<(f32, f32, f32, f32)>,
    sel: Vec<(f32, f32, f32, f32)>,
    shown: bool,
    req_id: u64,
}

impl CaretUi {
    /// Leave cursor mode and stop drawing the caret/selection. Bumping `req_id`
    /// drops any in-flight worker response.
    fn exit(&mut self) {
        self.mode = false;
        self.selecting = false;
        self.rect = None;
        self.sel.clear();
        self.shown = false;
        self.req_id += 1;
    }

    /// Forget everything on file switch (the worker resets its own caret on Load).
    fn reset(&mut self) {
        self.exit();
    }
}

/// Map a key event to a caret motion (Reader focus only). `None` for keys that
/// aren't caret commands (so they fall through to scroll/selection handling).
fn caret_key(ev: &Event) -> Option<CaretMotion> {
    let Event::Key(k) = ev else { return None };
    if k.kind != KeyEventKind::Press {
        return None;
    }
    // Option(⌥)+←/→ jumps/extends by word (the macOS word modifier; Cmd can't
    // reach a terminal app). Arrives as ALT via the standard CSI encoding.
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    Some(match k.code {
        KeyCode::Left if alt => CaretMotion::WordPrev,
        KeyCode::Right if alt => CaretMotion::WordNext,
        KeyCode::Left | KeyCode::Char('h') => CaretMotion::Left,
        KeyCode::Right | KeyCode::Char('l') => CaretMotion::Right,
        KeyCode::Up | KeyCode::Char('k') => CaretMotion::Up,
        KeyCode::Down | KeyCode::Char('j') => CaretMotion::Down,
        KeyCode::Char('w') => CaretMotion::WordNext,
        KeyCode::Char('b') => CaretMotion::WordPrev,
        KeyCode::Char('g') | KeyCode::Home => CaretMotion::DocStart,
        KeyCode::Char('G') | KeyCode::End => CaretMotion::DocEnd,
        KeyCode::Char('v') => CaretMotion::VisualStart,
        KeyCode::Char('y') | KeyCode::Enter => CaretMotion::Copy,
        _ => return None,
    })
}

/// The per-frame device-pixel geometry: the single source of truth for the
/// width-fit scale, the page's horizontal placement, and band slotting. Every
/// consumer (the band draw, `cell_to_page_px`, the selection overlay, and the
/// worker `Target`) reads the *same* `x_off`/`s`, so the sidebar offset can
/// never disagree across them.
struct Layout {
    /// Cells available to the page (terminal width minus the sidebar).
    content_w: u16,
    /// Viewport height in cells (terminal height minus the status row).
    view_h: u16,
    s: f32,
    scale_bits: u32,
    /// Page width in display pixels (`round(page_w · s)`).
    sw: u32,
    /// Page's left cell on screen — `sidebar_w` plus the centering slack within
    /// the content area, never less than `sidebar_w` (so the band can't bleed
    /// under the sidebar on a narrow terminal — it right-clips instead).
    x_off: u16,
    band_rows: u32,
    stride: u32,
    max_row0: u32,
    max_scroll: u32,
    target_row0: u32,
}

impl Layout {
    #[allow(clippy::too_many_arguments)]
    fn compute(
        term_w: u16,
        term_h: u16,
        sidebar_w: u16,
        page_w: u32,
        cell_w: u32,
        cell_h: u32,
        doc_total_h: u32,
        scroll_rows: u32,
    ) -> Self {
        let view_h = term_h.saturating_sub(1).max(1); // reserve status row
        let content_w = term_w.saturating_sub(sidebar_w).max(1);

        // Width-fit scale: largest (never magnified past full resolution) while
        // the page fits the *content* width. Independent of document length.
        let content_w_px = content_w as u32 * cell_w;
        let s = (content_w_px as f32 / page_w as f32).clamp(MIN_SCALE, 1.0);
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

        // Band geometry — stable (independent of document length) so a band's
        // identity doesn't change as the height estimate is refined.
        let sw = ((page_w as f32 * s).round() as u32).max(1);
        let band_rows = ((MAX_PIXELS / (sw as u64 * cell_h as u64)) as u32).max(view_h as u32 + 4);
        let max_row0 = doc_rows.saturating_sub(band_rows);
        // Adjacent bands overlap by at least a viewport, so any viewport fits
        // entirely inside exactly one slot.
        let stride = band_rows.saturating_sub(view_h as u32 + 2).max(1);
        let band_index = scroll_rows.min(max_scroll) / stride;
        let target_row0 = (band_index * stride).min(max_row0);

        let cols_disp = sw.div_ceil(cell_w).max(1) as u16;
        let x_off = sidebar_w + content_w.saturating_sub(cols_disp) / 2;
        let x_off = x_off.max(sidebar_w);

        Self {
            content_w,
            view_h,
            s,
            scale_bits,
            sw,
            x_off,
            band_rows,
            stride,
            max_row0,
            max_scroll,
            target_row0,
        }
    }
}

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// Switch the reader to a different file: read it, bump the document generation,
/// tell the worker to load it, and reset all per-document UI state so no stale
/// band, scroll position, or selection from the previous file survives. On a
/// read error the document is left untouched and the error is shown in the title.
#[allow(clippy::too_many_arguments)]
fn switch_to(
    path: &Path,
    req_tx: &Sender<Request>,
    doc_gen: &mut u64,
    cache: &mut Vec<CachedBand>,
    scroll_rows: &mut u32,
    doc_total_h: &mut u32,
    fully_shaped: &mut bool,
    last_target: &mut Option<Target>,
    selection: &mut Selection,
    overlay: &mut super::overlay::Overlay,
    title: &mut String,
    current_path: &mut Option<PathBuf>,
    caret: &mut CaretUi,
) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match std::fs::read_to_string(path) {
        Ok(content) => {
            *doc_gen += 1;
            let base_dir = path.parent().map(|p| p.to_path_buf());
            let _ = req_tx.send(Request::Load {
                epoch: *doc_gen,
                markdown: content,
                base_dir,
            });
            cache.clear();
            *scroll_rows = 0;
            *doc_total_h = 0;
            *fully_shaped = false;
            *last_target = None;
            // Invalidate any in-flight selection result for the old document.
            selection.req_id += 1;
            selection.clear();
            overlay.hide(&mut stdout());
            caret.reset();
            *title = name;
            *current_path = Some(path.to_path_buf());
        }
        Err(e) => {
            *title = format!("⚠ cannot open {name}: {e}");
        }
    }
}

/// Render the file-tree sidebar into `area`. Styled with the paper palette so it
/// reads as part of the page; the selected row is tinted (brighter when the
/// sidebar holds focus).
fn draw_sidebar(
    f: &mut ratatui::Frame,
    area: Rect,
    tree: &FileTree,
    style: &DocStyle,
    focused: bool,
) {
    let bg = rgb(style.bg);
    let ink = rgb(style.ink);
    let dim = rgb(style.chrome);
    let dir_fg = rgb(style.h[1]);
    f.render_widget(Block::default().style(Style::default().bg(bg)), area);

    let view_h = area.height as usize;
    let rows = tree.visible();
    let scroll = tree.scroll();
    let selected = tree.selected();
    let sel_bg = if focused {
        Color::Rgb(210, 224, 242)
    } else {
        Color::Rgb(232, 232, 235)
    };

    let mut lines: Vec<Line> = Vec::with_capacity(view_h);
    for i in 0..view_h {
        let idx = scroll + i;
        let Some(r) = rows.get(idx) else {
            lines.push(Line::from(""));
            continue;
        };
        let indent = "  ".repeat(r.depth as usize);
        let glyph = if r.is_dir {
            if r.expanded { "▾ " } else { "▸ " }
        } else {
            "  "
        };
        let name_style = if r.is_dir {
            Style::default().fg(dir_fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ink)
        };
        let mut spans = vec![
            Span::raw(indent),
            Span::styled(glyph, Style::default().fg(dim)),
            Span::styled(r.name.clone(), name_style),
        ];
        // Search results carry a dimmed locator ("parent · N matches").
        if let Some(detail) = &r.detail {
            spans.push(Span::styled(
                format!("  {detail}"),
                Style::default().fg(dim),
            ));
        }
        let mut line = Line::from(spans);
        if idx == selected {
            let mut st = Style::default().bg(sel_bg);
            if focused {
                st = st.add_modifier(Modifier::BOLD);
            }
            line = line.style(st);
        }
        lines.push(line);
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(bg).fg(ink)),
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    style: &DocStyle,
    page_w: u32,
    cell_w: u32,
    cell_h: u32,
    req_tx: &Sender<Request>,
    resp_rx: &Receiver<Response>,
    tree: &mut FileTree,
    mut title: String,
) -> Result<()> {
    let bg = rgb(style.bg);
    let mut scroll_rows: u32 = 0;
    let mut doc_total_h: u32 = 0; // unknown until the worker reports
    let mut fully_shaped = false;
    let mut cache: Vec<CachedBand> = Vec::new();
    let mut last_target: Option<Target> = None;
    let mut frame: u64 = 0;
    let mut dirty = true;
    let mut selection = Selection::default();
    let mut overlay = super::overlay::Overlay::default();
    let mut doc_gen: u64 = 0;
    let mut focus = Focus::Reader;
    let mut search = Search::Idle;
    let mut caret = CaretUi::default();
    // Full path of the file currently shown — so clearing a search can reveal it
    // back in the tree. Seeded from the tree's initial selection.
    let mut current_path: Option<PathBuf> = tree.selected_path().map(|p| p.to_path_buf());
    // Persistent sidebar whenever there are files to browse.
    let sidebar_enabled = !tree.is_empty();

    loop {
        let term = terminal.size()?;
        let (term_w, term_h) = (term.width, term.height);
        if term_w == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // Sidebar width: fixed, capped to a third of the terminal, and dropped
        // to 0 if it would leave too little room to read comfortably.
        let sidebar_w = if sidebar_enabled {
            let w = SIDEBAR_W.min(term_w / 3);
            if term_w.saturating_sub(w) < MIN_CONTENT_CELLS {
                0
            } else {
                w
            }
        } else {
            0
        };
        // If the sidebar collapsed away, keystrokes belong to the reader and any
        // active search is abandoned (its results would be invisible).
        if sidebar_w == 0 {
            focus = Focus::Reader;
            if search.is_active() {
                tree.clear_search(current_path.as_deref());
                search = Search::Idle;
                dirty = true;
            }
        }

        let layout = Layout::compute(
            term_w,
            term_h,
            sidebar_w,
            page_w,
            cell_w,
            cell_h,
            doc_total_h,
            scroll_rows,
        );
        let view_h = layout.view_h;
        let scale_bits = layout.scale_bits;
        let s = layout.s;
        let max_scroll = layout.max_scroll;
        if scroll_rows > max_scroll {
            scroll_rows = max_scroll;
        }
        tree.ensure_visible(view_h as usize);

        let target = Target {
            epoch: doc_gen,
            row0: layout.target_row0,
            rows: layout.band_rows,
            scale_bits,
            cell_w,
            cell_h,
            stride: layout.stride,
            max_row0: layout.max_row0,
        };

        // 1. Drain worker responses, discarding anything from an old document
        //    (epoch mismatch) so a late band can't flash stale content after a
        //    file switch — the fit-scale is identical across a switch, so
        //    `scale_bits` alone wouldn't catch it.
        loop {
            match resp_rx.try_recv() {
                Ok(Response::Progress {
                    epoch,
                    total_h,
                    fully_shaped: fs,
                }) => {
                    if epoch == doc_gen {
                        doc_total_h = total_h;
                        fully_shaped = fs;
                        dirty = true;
                    }
                }
                Ok(Response::Band {
                    epoch,
                    row0,
                    rows,
                    cols,
                    scale_bits,
                    proto,
                    total_h,
                    fully_shaped: fs,
                }) => {
                    if epoch == doc_gen {
                        doc_total_h = total_h;
                        fully_shaped = fs;
                        insert_band(
                            &mut cache,
                            CachedBand {
                                proto,
                                row0,
                                rows,
                                cols,
                                scale_bits,
                            },
                        );
                        dirty = true;
                    }
                }
                Ok(Response::Selection {
                    epoch,
                    id,
                    rects,
                    text,
                }) => {
                    // Only the latest request's result for the current document
                    // matters; drop stale ones (fast drag, or a prior file).
                    if epoch == doc_gen && id == selection.req_id {
                        selection.rects = rects;
                        if selection.copy_id == Some(id) {
                            super::clipboard::copy(&text);
                            selection.copy_id = None;
                        }
                        dirty = true;
                    }
                }
                Ok(Response::Caret {
                    epoch,
                    id,
                    caret: crect,
                    rects,
                    copy_text,
                }) => {
                    if epoch == doc_gen && id == caret.req_id {
                        caret.rect = crect;
                        caret.sel = rects;
                        caret.shown = crect.is_some();
                        if let Some(text) = copy_text {
                            super::clipboard::copy(&text);
                        }
                        // Scroll the page so the caret line stays visible.
                        if let Some((_, cy, _, ch)) = crect {
                            let s = layout.s;
                            let top = ((cy * s) / cell_h as f32).floor() as u32;
                            let bottom = (((cy + ch) * s) / cell_h as f32).ceil() as u32;
                            let margin = 2u32;
                            if top < scroll_rows + margin {
                                scroll_rows = top.saturating_sub(margin);
                            } else if bottom + margin > scroll_rows + view_h as u32 {
                                scroll_rows = (bottom + margin).saturating_sub(view_h as u32);
                            }
                            scroll_rows = scroll_rows.min(max_scroll);
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

        let x_off = layout.x_off;
        let content_w_px = layout.content_w as u32 * cell_w;

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
            // The status row doubles as the search bar while searching.
            let status = match &search {
                Search::Typing(q) => format!(" /{q}▏    ⏎ search · Esc cancel "),
                Search::Results { query, truncated } => {
                    let n = tree.visible().len();
                    let trunc = if *truncated { " · capped" } else { "" };
                    if n == 0 {
                        format!(" no matches for \"{query}\"{trunc}    Esc clear ")
                    } else {
                        format!(
                            " {n} match{} for \"{query}\"{trunc}    ↑/↓ · ⏎ open · Esc clear ",
                            if n == 1 { "" } else { "es" }
                        )
                    }
                }
                Search::Idle if caret.mode && caret.selecting => {
                    " SELECT · move · w/b by word · y copy · v stop · Esc cancel ".to_string()
                }
                Search::Idle if caret.mode => {
                    " CURSOR · arrows move · w/b word · v select · Esc exit ".to_string()
                }
                Search::Idle => {
                    let hint = match focus {
                        Focus::Sidebar => "↑/↓ select · ⏎ open · / find · Tab read · q quit",
                        Focus::Reader => "Tab files · / find · v cursor · j/k scroll · q quit",
                    };
                    let name = if title.is_empty() {
                        "reading"
                    } else {
                        title.as_str()
                    };
                    format!(" mdview · {name} · {pct}%{state}    {hint} ")
                }
            };
            let status_rect = Rect::new(0, term_h.saturating_sub(1), term_w, 1);
            let full = Rect::new(0, 0, term_w, view_h);
            let sidebar_focused = focus == Focus::Sidebar;

            let td = std::time::Instant::now();
            terminal.draw(|f| {
                f.render_widget(Block::default().style(Style::default().bg(bg)), full);
                // Sidebar first, then the band — disjoint column ranges, but
                // rendering the sidebar every frame lets ratatui's diff clear any
                // stale image cells at the seam after a band-narrowing resize.
                if sidebar_w > 0 {
                    let sb = Rect::new(0, 0, sidebar_w.saturating_sub(1), view_h);
                    draw_sidebar(f, sb, tree, style, sidebar_focused);
                    let div = Rect::new(sidebar_w - 1, 0, 1, view_h);
                    let bar = Paragraph::new(vec![Line::from("│"); view_h as usize])
                        .style(Style::default().fg(rgb(style.chrome)).bg(bg));
                    f.render_widget(bar, div);
                }
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
                    // Anchor at the unified x_off; clip width to the content area
                    // so the band can never overdraw the sidebar columns.
                    let img_w = b.cols.min(layout.content_w);
                    let img_area = Rect::new(x_off, 0, img_w, view_h);
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

            // Selection highlight + keyboard caret sit above the band as a
            // separate kitty placement; refreshed whenever the frame changed.
            // Width is clamped to the content area so a clipped narrow page
            // doesn't tint past it.
            let mut out = stdout();
            let overlay_w = layout.sw.min(content_w_px);
            // Mouse-drag and keyboard-visual highlights share one tint layer.
            let mut sel_rects = selection.rects.clone();
            sel_rects.extend_from_slice(&caret.sel);
            overlay.paint(
                &mut out,
                &sel_rects,
                caret.rect,
                x_off,
                scroll_rows,
                cell_h,
                view_h,
                overlay_w,
                s,
            );
        }

        // 5. Input. Poll faster while waiting for the covering band to arrive.
        let page = view_h.saturating_sub(2).max(1) as u32;
        let poll_ms = if covered { EVENT_POLL_MS } else { WAIT_POLL_MS };
        if event::poll(Duration::from_millis(poll_ms))? {
            for _ in 0..MAX_EVENTS_PER_FRAME {
                let ev = event::read()?;

                // While the search bar is open, key events build the query and
                // shadow everything else (so `q`, `j/k`, `Tab` are literal text).
                // Non-key events (mouse/resize) still fall through below.
                if search.is_typing()
                    && let Event::Key(k) = &ev
                {
                    if k.kind == KeyEventKind::Press {
                        let Search::Typing(mut q) = std::mem::replace(&mut search, Search::Idle)
                        else {
                            unreachable!("is_typing() checked above")
                        };
                        match k.code {
                            KeyCode::Esc => {
                                if tree.is_searching() {
                                    tree.clear_search(current_path.as_deref());
                                }
                                // search stays Idle (cancelled).
                            }
                            KeyCode::Enter => {
                                if q.trim().is_empty() {
                                    if tree.is_searching() {
                                        tree.clear_search(current_path.as_deref());
                                    }
                                } else {
                                    let outcome = tree.set_search(&q);
                                    focus = Focus::Sidebar;
                                    search = Search::Results {
                                        query: q,
                                        truncated: outcome.truncated,
                                    };
                                }
                            }
                            KeyCode::Backspace => {
                                q.pop();
                                search = Search::Typing(q);
                            }
                            KeyCode::Char(c) if !c.is_control() => {
                                q.push(c);
                                search = Search::Typing(q);
                            }
                            _ => search = Search::Typing(q), // unchanged
                        }
                        dirty = true;
                    }
                    // Key consumed by the search bar.
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                    continue;
                }

                // Global keys (regardless of focus): quit, focus toggle, search.
                // Resize just needs a redraw.
                let mut handled = false;
                match &ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                        // Esc unwinds in order: close search, else hide the
                        // caret, else quit.
                        KeyCode::Esc if search.is_active() => {
                            tree.clear_search(current_path.as_deref());
                            search = Search::Idle;
                            dirty = true;
                            handled = true;
                        }
                        KeyCode::Esc if caret.mode => {
                            caret.exit();
                            let _ = req_tx.send(Request::Caret {
                                id: caret.req_id,
                                motion: CaretMotion::Hide,
                                top: 0,
                            });
                            dirty = true;
                            handled = true;
                        }
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('/') if sidebar_w > 0 => {
                            search = Search::Typing(String::new());
                            dirty = true;
                            handled = true;
                        }
                        KeyCode::Tab | KeyCode::BackTab => {
                            if sidebar_w > 0 {
                                focus = match focus {
                                    Focus::Reader => Focus::Sidebar,
                                    Focus::Sidebar => Focus::Reader,
                                };
                                dirty = true;
                            }
                            handled = true;
                        }
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        dirty = true;
                        handled = true;
                    }
                    _ => {}
                }

                // Mouse is routed by column, independent of keyboard focus: clicks
                // and wheel over the sidebar drive the tree; elsewhere the reader.
                if !handled
                    && let Event::Mouse(m) = &ev
                    && sidebar_w > 0
                    && m.column < sidebar_w
                {
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            if let Some(idx) = tree.row_at(m.row as usize) {
                                tree.select_index(idx);
                                focus = Focus::Sidebar;
                                if let Some(path) = tree.activate() {
                                    switch_to(
                                        &path,
                                        req_tx,
                                        &mut doc_gen,
                                        &mut cache,
                                        &mut scroll_rows,
                                        &mut doc_total_h,
                                        &mut fully_shaped,
                                        &mut last_target,
                                        &mut selection,
                                        &mut overlay,
                                        &mut title,
                                        &mut current_path,
                                        &mut caret,
                                    );
                                }
                                dirty = true;
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            tree.select_next();
                            dirty = true;
                        }
                        MouseEventKind::ScrollUp => {
                            tree.select_prev();
                            dirty = true;
                        }
                        _ => {}
                    }
                    handled = true;
                }

                // Keyboard while the sidebar is focused drives tree navigation;
                // everything else (reader keys, all content-area mouse) goes to
                // the reader's selection + scroll handling.
                if !handled {
                    let sidebar_key = focus == Focus::Sidebar
                        && matches!(&ev, Event::Key(k) if k.kind == KeyEventKind::Press);
                    if sidebar_key {
                        if let Event::Key(k) = &ev {
                            match k.code {
                                KeyCode::Down | KeyCode::Char('j') => {
                                    tree.select_next();
                                    dirty = true;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    tree.select_prev();
                                    dirty = true;
                                }
                                KeyCode::Enter
                                | KeyCode::Char(' ')
                                | KeyCode::Right
                                | KeyCode::Left => {
                                    if let Some(path) = tree.activate() {
                                        switch_to(
                                            &path,
                                            req_tx,
                                            &mut doc_gen,
                                            &mut cache,
                                            &mut scroll_rows,
                                            &mut doc_total_h,
                                            &mut fully_shaped,
                                            &mut last_target,
                                            &mut selection,
                                            &mut overlay,
                                            &mut title,
                                            &mut current_path,
                                            &mut caret,
                                        );
                                    }
                                    dirty = true;
                                }
                                _ => {}
                            }
                        }
                    } else if focus == Focus::Reader
                        && !caret.mode
                        && matches!(&ev, Event::Key(k)
                            if k.kind == KeyEventKind::Press && k.code == KeyCode::Char('v'))
                    {
                        // Opt-in: `v` enters cursor mode at the top of the viewport.
                        // Reading keys keep scrolling until then.
                        caret.mode = true;
                        caret.selecting = false;
                        caret.req_id += 1;
                        let top = (scroll_rows as f32 * cell_h as f32 / s).max(0.0) as u32;
                        let _ = req_tx.send(Request::Caret {
                            id: caret.req_id,
                            motion: CaretMotion::Show,
                            top,
                        });
                        dirty = true;
                    } else if focus == Focus::Reader
                        && caret.mode
                        && let Some(motion) = caret_key(&ev)
                    {
                        // In cursor mode: caret keys move/select; the view follows.
                        caret.req_id += 1;
                        let top = (scroll_rows as f32 * cell_h as f32 / s).max(0.0) as u32;
                        let _ = req_tx.send(Request::Caret {
                            id: caret.req_id,
                            motion,
                            top,
                        });
                        match motion {
                            CaretMotion::VisualStart => caret.selecting = !caret.selecting,
                            CaretMotion::Copy => {
                                // Copy exits cursor mode; the response clears the
                                // caret + does the clipboard write.
                                caret.mode = false;
                                caret.selecting = false;
                                caret.rect = None;
                                caret.sel.clear();
                                caret.shown = false;
                            }
                            _ => {}
                        }
                        dirty = true;
                    } else if handle_selection(
                        &ev,
                        &mut selection,
                        req_tx,
                        x_off,
                        scroll_rows,
                        cell_w,
                        cell_h,
                        s,
                    ) {
                        dirty = true;
                    } else {
                        match handle_event(ev, &mut scroll_rows, max_scroll, page) {
                            RichEvent::Quit => return Ok(()),
                            RichEvent::Changed => dirty = true,
                            RichEvent::Ignored => {}
                        }
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
    let point = || cell_to_page_px(m.column, m.row, x_off, scroll_rows, cell_w, cell_h, s);
    match m.kind {
        // Only *start* a selection inside the page (at or right of `x_off`); a
        // press in the left margin / gutter isn't a text selection. A drag that
        // began in the page, though, keeps tracking even if it crosses left —
        // `cell_to_page_px` clamps the head to page-x 0 — so it never wedges.
        MouseEventKind::Down(MouseButton::Left) => {
            if m.column < x_off {
                return false;
            }
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
            epoch: 0,
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
                Ok(Response::Band {
                    rows,
                    cols,
                    total_h,
                    scale_bits,
                    row0,
                    ..
                }) => {
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

    /// After a `Load`, the worker must serve the *new* document and stamp every
    /// response with the new generation — never the old one. This is the guard
    /// against a stale band from the previous file flashing on screen (the
    /// fit-scale is identical across a switch, so the epoch is the only signal).
    #[test]
    fn worker_switches_document_and_stamps_epoch() {
        let md = sample_markdown(20);
        let style = DocStyle::light();
        let (cell_w, cell_h) = (8u32, 16u32);
        let scale = 0.5f32;
        let mk_target = |epoch: u64| Target {
            epoch,
            row0: 0,
            rows: 80,
            scale_bits: scale.to_bits(),
            cell_w,
            cell_h,
            stride: 60,
            max_row0: 10_000,
        };

        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();
        let handle = {
            let style = style.clone();
            let picker = headless_picker();
            std::thread::spawn(move || render_worker(md, style, picker, None, req_rx, resp_tx))
        };

        // Serve the first document (epoch 0).
        req_tx.send(Request::SetTarget(mk_target(0))).unwrap();
        // Wait for a band of the wanted epoch. Bands from an *older* epoch may
        // still be in the channel (e.g. a prefetched neighbor enqueued before
        // the `Load` was drained) — those are exactly what the UI drops, so we
        // skip them here. A band from a *newer* epoch would be a real bug.
        let wait_band = |epoch_want: u64| {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while std::time::Instant::now() < deadline {
                match resp_rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(Response::Band { epoch, .. }) => {
                        assert!(epoch <= epoch_want, "band stamped with a future epoch");
                        if epoch == epoch_want {
                            return true;
                        }
                    }
                    Ok(_) => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                }
            }
            false
        };
        assert!(wait_band(0), "no band for the initial document");

        // Switch documents (epoch 1) and ask for the new top band.
        req_tx
            .send(Request::Load {
                epoch: 1,
                markdown: sample_markdown(5),
                base_dir: None,
            })
            .unwrap();
        req_tx.send(Request::SetTarget(mk_target(1))).unwrap();
        assert!(
            wait_band(1),
            "worker did not serve the switched-to document with the new epoch"
        );

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
            epoch: 0,
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
        assert!(
            resp.is_ok(),
            "worker should respond even for an out-of-range target"
        );

        req_tx.send(Request::Quit).unwrap();
        handle.join().expect("worker thread should exit cleanly");
    }

    #[test]
    fn neighbors_are_clamped_and_distinct() {
        // Middle band: both neighbors valid and distinct.
        let t = Target {
            epoch: 0,
            row0: 100,
            rows: 80,
            scale_bits: 0,
            cell_w: 8,
            cell_h: 16,
            stride: 60,
            max_row0: 500,
        };
        let mut n = neighbor_row0s(&t);
        n.sort_unstable();
        assert_eq!(n, vec![40, 160]);

        // Top band: no upward neighbor.
        let t = Target {
            epoch: 0,
            row0: 0,
            rows: 80,
            scale_bits: 0,
            cell_w: 8,
            cell_h: 16,
            stride: 60,
            max_row0: 500,
        };
        assert_eq!(neighbor_row0s(&t), vec![60]);

        // Bottom band (row0 == max_row0): downward neighbor clamps away, only up.
        let t = Target {
            epoch: 0,
            row0: 500,
            rows: 80,
            scale_bits: 0,
            cell_w: 8,
            cell_h: 16,
            stride: 60,
            max_row0: 500,
        };
        assert_eq!(neighbor_row0s(&t), vec![440]);
    }

    #[test]
    fn cache_is_lru_with_dedup() {
        let mk = |row0: u32| CachedBand {
            // A trivial 1x1 protocol via the headless picker.
            proto: SlicedProtocol::new(
                &headless_picker(),
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    8,
                    16,
                    image::Rgba([255, 255, 255, 255]),
                )),
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
        assert_eq!(
            cache.last().unwrap().row0,
            keep,
            "re-inserted band is newest"
        );
    }
}
