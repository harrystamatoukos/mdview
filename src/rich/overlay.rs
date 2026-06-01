//! Selection-highlight overlay, drawn as a *classic* kitty graphics placement
//! layered above the document band.
//!
//! The band is rendered via kitty Unicode placeholders (one image id per cell),
//! so a second image can't share those cells through that path. Instead the
//! highlight is a separate, translucent RGBA image transmitted with a classic
//! placement at `z=1` — kitty composites it (alpha-blended) on top of the band.
//! Re-using the same image id each frame replaces the placement without flicker,
//! which is what makes a live drag-highlight smooth.
//!
//! The bytes are written straight to stdout *after* ratatui's draw, since the
//! overlay isn't part of ratatui's cell buffer.
//!
//! Not yet handled: tmux passthrough wrapping (the band path handles its own).

use std::fmt::Write as _;
use std::io::Write;

use image::{Rgba, RgbaImage};

use super::clipboard::base64_encode;

/// Fixed image id for the overlay — large enough to avoid colliding with the
/// small ids the band picker hands out.
const OVERLAY_ID: u32 = 0x7FFF_FF00;

/// Translucent selection tint (RGBA), alpha-blended over the page by kitty.
const TINT: Rgba<u8> = Rgba([74, 144, 226, 90]);
/// Opaque keyboard-caret bar (RGBA) — a clearly visible cursor over the page.
const CARET: Rgba<u8> = Rgba([26, 95, 180, 255]);
/// Faint full-width band on the caret's line, so the cursor is easy to track.
const CARET_LINE: Rgba<u8> = Rgba([74, 144, 226, 26]);

/// Manages the lifecycle of the on-screen highlight placement.
#[derive(Default)]
pub struct Overlay {
    shown: bool,
}

impl Overlay {
    /// Draw (or update) the highlight for `rects` — selection rectangles in page
    /// **device pixels** `(x, y, w, h)`. `x_off` is the page's left cell on
    /// screen; `scroll_rows` the viewport top in display cell-rows; `s` the
    /// display scale; `view_h` the viewport height in cells. Builds a viewport-
    /// sized translucent image, positions it over the band, and transmits it.
    #[allow(clippy::too_many_arguments)]
    pub fn paint<W: Write>(
        &mut self,
        out: &mut W,
        rects: &[(f32, f32, f32, f32)],
        caret: Option<(f32, f32, f32, f32)>,
        x_off: u16,
        scroll_rows: u32,
        cell_h: u32,
        view_h: u16,
        page_w_disp: u32,
        s: f32,
    ) {
        if rects.is_empty() && caret.is_none() {
            self.hide(out);
            return;
        }

        let w = page_w_disp.max(1);
        let h = (view_h as u32 * cell_h).max(1);
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));

        // Display-pixel offset of the viewport top within the document.
        let scroll_px = (scroll_rows * cell_h) as f32;
        let fill = |img: &mut RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32, c: Rgba<u8>| {
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    img.put_pixel(x, y, c);
                }
            }
        };
        // Caret line dimensions (display px → viewport-local), reused below.
        let caret_rows = caret.map(|(cx, cy, _cw, ch)| {
            let x0 = (cx * s).floor().max(0.0) as u32;
            let y0 = (cy * s - scroll_px).floor().clamp(0.0, h as f32) as u32;
            let y1 = ((cy + ch) * s - scroll_px).ceil().clamp(0.0, h as f32) as u32;
            (x0, y0, y1)
        });

        // 1. Faint full-width band on the caret's line (under the selection, so a
        //    highlight on that line still reads strongly).
        if let Some((_, y0, y1)) = caret_rows {
            fill(&mut img, 0, w, y0, y1, CARET_LINE);
        }

        // 2. Selection highlight.
        for &(rx, ry, rw, rh) in rects {
            // device px → display px, then into viewport-local coordinates.
            let x0 = (rx * s).floor().max(0.0) as u32;
            let y0f = ry * s - scroll_px;
            let x1 = ((rx + rw) * s).ceil() as u32;
            let y1f = (ry + rh) * s - scroll_px;
            let y0 = y0f.floor().clamp(0.0, h as f32) as u32;
            let y1 = y1f.ceil().clamp(0.0, h as f32) as u32;
            fill(&mut img, x0, x1, y0, y1, TINT);
        }

        // 3. Caret bar on top (opaque, ≥3 display px so it never vanishes).
        if let Some((x0, y0, y1)) = caret_rows {
            let bar = (3.0 * s).round().max(3.0) as u32;
            fill(&mut img, x0, x0 + bar, y0, y1, CARET);
        }

        let seq = transmit_classic(&img, OVERLAY_ID);
        // Move the cursor to the page's top-left cell, emit the placement (which
        // displays at the cursor without moving it, C=1), then we're done.
        let mut buf = String::with_capacity(seq.len() + 16);
        // 1-based; row 1 is the top of the viewport.
        let _ = write!(buf, "\x1b[1;{}H", x_off + 1);
        buf.push_str(&seq);
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
        self.shown = true;
    }

    /// Remove the highlight from the screen, if shown.
    pub fn hide<W: Write>(&mut self, out: &mut W) {
        if !self.shown {
            return;
        }
        // a=d,d=i deletes placements of this image id (keeps transmitted data).
        let _ = out.write_all(format!("\x1b_Ga=d,d=i,i={OVERLAY_ID}\x1b\\").as_bytes());
        let _ = out.flush();
        self.shown = false;
    }
}

/// Build a transmit-and-place escape sequence for `img` at the current cursor,
/// `z=1` (above text), `C=1` (don't move the cursor). Pixel data is zlib
/// (`o=z`) compressed and base64-chunked, mirroring the band transmit.
fn transmit_classic(img: &RgbaImage, id: u32) -> String {
    let (w, h) = (img.width(), img.height());
    let raw = img.as_raw();
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(raw, 2);
    let (bytes, zlib): (&[u8], bool) = if compressed.len() < raw.len() {
        (&compressed, true)
    } else {
        (raw, false)
    };

    // 4096 base64 chars per chunk → 3072 raw bytes.
    const CHUNK: usize = (4096 / 4) * 3;
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    let n = chunks.len();
    let mut data = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        data.push_str("\x1b_G");
        if i == 0 {
            let o = if zlib { "o=z," } else { "" };
            // `p=1` reuses a single placement so each drag tick *replaces* the
            // overlay (flicker-free) instead of stacking a new placement.
            let _ = write!(data, "q=2,i={id},p=1,a=T,f=32,t=d,z=1,C=1,{o}s={w},v={h},");
        }
        let more = u8::from(n > i + 1);
        let _ = write!(data, "m={more};");
        data.push_str(&base64_encode(chunk));
        data.push_str("\x1b\\");
    }
    data
}
