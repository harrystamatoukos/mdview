//! Native Mermaid **flowchart** rasterization for the rich reader.
//!
//! Parses are done in [`crate::mermaid`]; this module lays the graph out
//! (longest-path layered layout) and draws it straight into an `RgbaImage` via
//! `plotters` — the same in-memory-bitmap → image-blit path charts and tables
//! use. No browser, no new dependency.
//!
//! Layout is intentionally simple (Sugiyama-lite): rank by longest path, order
//! within a rank by appearance, center each rank, connect box borders with
//! straight arrows. Good enough for the small flowcharts that appear in
//! markdown; not a general graph-drawing engine.

use image::{Rgba, RgbaImage};
use plotters::element::Polygon;
use plotters::prelude::*;
use plotters::style::RGBColor;

use super::paint::{DocStyle, Rgb};
use crate::mermaid::{Dir, Flowchart, Shape};

fn color(rgb: Rgb) -> RGBColor {
    RGBColor(rgb.0, rgb.1, rgb.2)
}

fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// A laid-out node: center + half-extents, all in natural (pre-scale) px.
#[derive(Clone, Copy)]
struct Box2 {
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
}

impl Box2 {
    /// Point on the box border along the ray toward `(tx, ty)`.
    fn border_toward(&self, tx: f32, ty: f32) -> (f32, f32) {
        let dx = tx - self.cx;
        let dy = ty - self.cy;
        if dx == 0.0 && dy == 0.0 {
            return (self.cx, self.cy);
        }
        let sx = if dx != 0.0 {
            self.hw / dx.abs()
        } else {
            f32::INFINITY
        };
        let sy = if dy != 0.0 {
            self.hh / dy.abs()
        } else {
            f32::INFINITY
        };
        let t = sx.min(sy);
        (self.cx + dx * t, self.cy + dy * t)
    }
}

/// Rasterize `fc` to an `RgbaImage` `width` device-px wide. Best-effort: any
/// drawing error yields whatever has been drawn so far on the themed page
/// background rather than failing the page.
pub fn render_flowchart(fc: &Flowchart, width: u32, st: &DocStyle) -> RgbaImage {
    super::chart_render::ensure_font();
    let width = width.max(64);

    // ── Sizing ───────────────────────────────────────────────────────────
    let font_px = st.base_px * 0.6;
    let pad = st.base_px * 0.6;
    let rank_gap = st.base_px * 1.4;
    let cross_gap = st.base_px * 0.8;

    let node_box = |label: &str, shape: Shape| -> (f32, f32) {
        let tw = label.chars().count().max(1) as f32 * font_px * 0.6;
        let mut w = tw + font_px * 1.6;
        let mut h = font_px + font_px * 1.2;
        w = w.max(font_px * 3.0);
        match shape {
            Shape::Diamond => {
                w *= 1.35;
                h *= 1.55;
            }
            Shape::Circle => {
                let d = w.max(h * 1.6);
                w = d;
                h = d;
            }
            _ => {}
        }
        (w, h)
    };

    let ranks = fc.ranks();
    let n_ranks = ranks.iter().copied().max().unwrap_or(0) + 1;
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); n_ranks];
    for (i, &r) in ranks.iter().enumerate() {
        groups[r].push(i);
    }

    // Box dimensions per node.
    let dims: Vec<(f32, f32)> = fc
        .nodes
        .iter()
        .map(|nd| node_box(&nd.label, nd.shape))
        .collect();

    let vertical = fc.dir.is_vertical();
    // "main" = along the flow direction; "cross" = perpendicular.
    let main_of = |d: (f32, f32)| if vertical { d.1 } else { d.0 };
    let cross_of = |d: (f32, f32)| if vertical { d.0 } else { d.1 };

    // Per-rank main size and cross extent.
    let mut main_size = vec![0f32; n_ranks];
    let mut cross_ext = vec![0f32; n_ranks];
    for (r, group) in groups.iter().enumerate() {
        let mut cross = 0.0;
        for (k, &ni) in group.iter().enumerate() {
            main_size[r] = main_size[r].max(main_of(dims[ni]));
            cross += cross_of(dims[ni]);
            if k + 1 < group.len() {
                cross += cross_gap;
            }
        }
        cross_ext[r] = cross;
    }

    let canvas_main: f32 =
        main_size.iter().sum::<f32>() + rank_gap * (n_ranks.max(1) - 1) as f32 + pad * 2.0;
    let canvas_cross: f32 = cross_ext.iter().cloned().fold(0.0, f32::max) + pad * 2.0;

    // ── Position each node in the natural canvas ─────────────────────────
    let mut boxes: Vec<Box2> = vec![
        Box2 {
            cx: 0.0,
            cy: 0.0,
            hw: 0.0,
            hh: 0.0
        };
        fc.nodes.len()
    ];
    let mut main_off = pad;
    for (r, group) in groups.iter().enumerate() {
        let band = main_size[r];
        let mut cross_cur = pad + (canvas_cross - 2.0 * pad - cross_ext[r]) / 2.0;
        for &ni in group {
            let (bw, bh) = dims[ni];
            let main_center = main_off + band / 2.0;
            let cross_center = cross_cur + cross_of((bw, bh)) / 2.0;
            let (mut cx, mut cy);
            if vertical {
                cx = cross_center;
                cy = main_center;
            } else {
                cx = main_center;
                cy = cross_center;
            }
            // Up / Left invert the main axis so rank 0 sits at the bottom/right.
            match fc.dir {
                Dir::Up => cy = canvas_main - cy,
                Dir::Left => cx = canvas_main - cx,
                _ => {}
            }
            boxes[ni] = Box2 {
                cx,
                cy,
                hw: bw / 2.0,
                hh: bh / 2.0,
            };
            cross_cur += cross_of((bw, bh)) + cross_gap;
        }
        main_off += band + rank_gap;
    }

    let (canvas_w, canvas_h) = if vertical {
        (canvas_cross, canvas_main)
    } else {
        (canvas_main, canvas_cross)
    };

    // ── Fit to target width (scale down if needed, else center) ──────────
    let (scale, x_off) = if canvas_w > width as f32 {
        (width as f32 / canvas_w, 0.0)
    } else {
        (1.0, (width as f32 - canvas_w) / 2.0)
    };
    let height = (canvas_h * scale).ceil().max(1.0) as u32;
    let sfont = font_px * scale;

    let mut buf = vec![0u8; (width * height * 3) as usize];
    let _ = draw(fc, &boxes, st, &mut buf, width, height, scale, x_off, sfont);
    rgb_to_rgba(&buf, width, height)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    fc: &Flowchart,
    boxes: &[Box2],
    st: &DocStyle,
    buf: &mut [u8],
    width: u32,
    height: u32,
    scale: f32,
    x_off: f32,
    sfont: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = BitMapBackend::with_buffer(buf, (width, height)).into_drawing_area();
    root.fill(&color(st.bg))?;

    let fill = color(blend(st.bg, st.link, 0.10));
    let border = color(blend(st.ink, st.bg, 0.5));
    let edge_col = color(st.accent);
    let ink = color(st.ink);
    let stroke = (st.base_px * 0.05 * scale).max(1.5) as u32;

    // Transform a natural-canvas point to final pixels.
    let tx = |x: f32| (x * scale + x_off).round() as i32;
    let ty = |y: f32| (y * scale).round() as i32;

    // ── Edges (drawn first, so node boxes sit on top of the line ends) ───
    for e in &fc.edges {
        let a = &boxes[e.from];
        let b = &boxes[e.to];
        let (sx, sy) = a.border_toward(b.cx, b.cy);
        let (ex, ey) = b.border_toward(a.cx, a.cy);
        let line_col = if e.dashed {
            edge_col.mix(0.5)
        } else {
            edge_col.to_rgba()
        };
        root.draw(&PathElement::new(
            vec![(tx(sx), ty(sy)), (tx(ex), ty(ey))],
            line_col.stroke_width(stroke),
        ))?;

        if e.arrow {
            draw_arrowhead(&root, (sx, sy), (ex, ey), sfont, edge_col, &tx, &ty)?;
        }

        if let Some(label) = &e.label {
            let mx = (sx + ex) / 2.0;
            let my = (sy + ey) / 2.0;
            let lw = label.chars().count() as f32 * sfont * 0.6;
            // Knock out the line behind the label for legibility.
            root.draw(&Rectangle::new(
                [
                    (
                        tx(mx) - (lw / 2.0) as i32 - 4,
                        ty(my) - (sfont * 0.7) as i32,
                    ),
                    (
                        tx(mx) + (lw / 2.0) as i32 + 4,
                        ty(my) + (sfont * 0.7) as i32,
                    ),
                ],
                color(st.bg).filled(),
            ))?;
            root.draw(&Text::new(
                label.clone(),
                (tx(mx) - (lw / 2.0) as i32, ty(my) - (sfont * 0.55) as i32),
                ("sans-serif", (sfont * 0.92) as i32)
                    .into_font()
                    .color(&edge_col),
            ))?;
        }
    }

    // ── Nodes ────────────────────────────────────────────────────────────
    for (i, nd) in fc.nodes.iter().enumerate() {
        let bx = &boxes[i];
        let (x0, y0) = (tx(bx.cx - bx.hw), ty(bx.cy - bx.hh));
        let (x1, y1) = (tx(bx.cx + bx.hw), ty(bx.cy + bx.hh));

        match nd.shape {
            Shape::Diamond => {
                let pts = vec![
                    (tx(bx.cx), y0),
                    (x1, ty(bx.cy)),
                    (tx(bx.cx), y1),
                    (x0, ty(bx.cy)),
                ];
                root.draw(&Polygon::new(pts.clone(), fill.filled()))?;
                let mut outline = pts;
                outline.push(outline[0]);
                root.draw(&PathElement::new(outline, border.stroke_width(stroke)))?;
            }
            Shape::Circle => {
                let r = ((x1 - x0).min(y1 - y0) / 2).max(1);
                let c = (tx(bx.cx), ty(bx.cy));
                root.draw(&Circle::new(c, r, fill.filled()))?;
                root.draw(&Circle::new(c, r, border.stroke_width(stroke)))?;
            }
            _ => {
                root.draw(&Rectangle::new([(x0, y0), (x1, y1)], fill.filled()))?;
                root.draw(&Rectangle::new(
                    [(x0, y0), (x1, y1)],
                    border.stroke_width(stroke),
                ))?;
            }
        }

        // Centered label.
        let lw = nd.label.chars().count() as f32 * sfont * 0.6;
        root.draw(&Text::new(
            nd.label.clone(),
            (
                tx(bx.cx) - (lw / 2.0) as i32,
                ty(bx.cy) - (sfont * 0.6) as i32,
            ),
            ("sans-serif", sfont as i32).into_font().color(&ink),
        ))?;
    }

    root.present()?;
    Ok(())
}

/// Draw a filled triangular arrowhead at `end`, pointing along `start → end`.
#[allow(clippy::too_many_arguments)]
fn draw_arrowhead<DB>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    start: (f32, f32),
    end: (f32, f32),
    sfont: f32,
    col: RGBColor,
    tx: &dyn Fn(f32) -> i32,
    ty: &dyn Fn(f32) -> i32,
) -> Result<(), Box<dyn std::error::Error>>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (-uy, ux); // perpendicular
    let a = sfont * 0.8; // arrowhead length
    let w = a * 0.55; // half-width
    let base = (end.0 - ux * a, end.1 - uy * a);
    let left = (base.0 + px * w, base.1 + py * w);
    let right = (base.0 - px * w, base.1 - py * w);
    root.draw(&Polygon::new(
        vec![
            (tx(end.0), ty(end.1)),
            (tx(left.0), ty(left.1)),
            (tx(right.0), ty(right.1)),
        ],
        col.filled(),
    ))?;
    Ok(())
}

/// Lift plotters' RGB output to an opaque RGBA image.
fn rgb_to_rgba(rgb: &[u8], width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::new(width, height);
    for (i, px) in img.pixels_mut().enumerate() {
        let o = i * 3;
        *px = Rgba([rgb[o], rgb[o + 1], rgb[o + 2], 255]);
    }
    img
}
