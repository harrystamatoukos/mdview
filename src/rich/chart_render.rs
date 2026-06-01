//! Native chart rasterization for the rich reader, via `plotters`.
//!
//! Renders a parsed [`Chart`] into an `RgbaImage` sized to the content column,
//! themed from the document's [`DocStyle`] so charts sit in the page alongside
//! the prose. No browser, no external process — `plotters` draws straight into
//! an in-memory RGB buffer which we lift to RGBA for the existing image-blit
//! path (same mechanism tables use).

use std::sync::OnceLock;

use image::{Rgba, RgbaImage};
use plotters::prelude::*;
use plotters::style::{FontStyle, RGBColor};

use crate::chart::{Chart, ChartKind, fmt_num};
use super::paint::{DocStyle, Rgb};

/// plotters' pure-Rust `ab_glyph` backend ships no default font, so we register
/// one ourselves (once). We reuse a real `.ttf` the system already has — the
/// rich reader already depends on these on macOS — and fall back across
/// platforms. Without this, every text draw fails with `FontUnavailable` and
/// the whole chart comes out blank.
fn ensure_font() -> bool {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    *REGISTERED.get_or_init(|| {
        const CANDIDATES: &[&str] = &[
            // macOS — Georgia is the reader's own body font; guaranteed present
            // wherever the rich reader runs.
            "/System/Library/Fonts/Supplemental/Georgia.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            // Linux
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            // Windows
            "C:\\Windows\\Fonts\\arial.ttf",
        ];
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                // We only ever draw the Normal style, under the "sans-serif" key.
                if plotters::style::register_font("sans-serif", FontStyle::Normal, leaked).is_ok() {
                    return true;
                }
            }
        }
        false
    })
}

/// Series palette (cycled). Distinct, readable hues that work on a light page.
const PALETTE: [Rgb; 6] = [
    (26, 95, 180),   // blue
    (200, 80, 60),   // red
    (40, 140, 90),   // green
    (180, 130, 30),  // amber
    (120, 70, 170),  // purple
    (60, 150, 170),  // teal
];

fn color(rgb: Rgb) -> RGBColor {
    RGBColor(rgb.0, rgb.1, rgb.2)
}

/// Rasterize `chart` to an `RgbaImage` `width` device-pixels wide. Best-effort:
/// any plotters error yields a blank themed panel rather than failing the page.
pub fn render_chart(chart: &Chart, width: u32, st: &DocStyle) -> RgbaImage {
    let width = width.max(64);
    let height = ((width as f32) * 0.6) as u32;

    // plotters writes RGB (3 bytes/px); we convert to RGBA afterward.
    ensure_font();
    let mut buf = vec![0u8; (width * height * 3) as usize];
    let _ = draw(chart, st, &mut buf, width, height);
    rgb_to_rgba(&buf, width, height)
}

/// Inner draw, fallible so plotters' `?` works; the caller ignores errors.
fn draw(
    chart: &Chart,
    st: &DocStyle,
    buf: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = BitMapBackend::with_buffer(buf, (width, height)).into_drawing_area();
    root.fill(&color(st.bg))?;

    let ink = color(st.ink);
    let axis = color(st.chrome);
    let grid = color(st.chrome);
    let title_px = (st.base_px * 0.9) as i32;
    let label_px = (st.base_px * 0.62) as i32;
    let margin = (st.base_px * 0.4) as i32;
    let x_area = (st.base_px * 1.4) as i32;
    let y_area = (st.base_px * 1.8) as i32;
    let caption = chart.title.clone().unwrap_or_default();
    let title_font = || ("sans-serif", title_px).into_font().color(&ink);
    let label_font = || ("sans-serif", label_px).into_font().color(&ink);

    match chart.kind {
        ChartKind::Pie => {
            draw_pie(chart, st, &root, ink, title_px)?;
        }

        ChartKind::Scatter => {
            let (xr, yr) = scatter_ranges(chart);
            let mut cc = ChartBuilder::on(&root)
                .caption(caption, title_font())
                .margin(margin)
                .x_label_area_size(x_area)
                .y_label_area_size(y_area)
                .build_cartesian_2d(xr, yr)?;
            cc.configure_mesh()
                .axis_style(axis)
                .light_line_style(grid.mix(0.35))
                .bold_line_style(grid.mix(0.6))
                .label_style(label_font())
                .x_desc(chart.xlabel.clone().unwrap_or_default())
                .y_desc(chart.ylabel.clone().unwrap_or_default())
                .draw()?;
            cc.draw_series(chart.points.iter().map(|&(x, y)| {
                Circle::new((x, y), (label_px / 4).max(2), color(PALETTE[0]).filled())
            }))?;
        }

        ChartKind::Bar => {
            let n = chart.x.len().max(longest_series(chart));
            let ymax = chart.y_max() * 1.15;
            let labels = chart.x.clone();
            let mut cc = ChartBuilder::on(&root)
                .caption(caption, title_font())
                .margin(margin)
                .x_label_area_size(x_area)
                .y_label_area_size(y_area)
                .build_cartesian_2d(-0.5f64..(n as f64 - 0.5), 0f64..ymax)?;
            cc.configure_mesh()
                .axis_style(axis)
                .light_line_style(grid.mix(0.0))
                .bold_line_style(grid.mix(0.5))
                .label_style(label_font())
                .x_labels(n.max(1))
                .x_label_formatter(&|v: &f64| label_at(&labels, *v))
                .x_desc(chart.xlabel.clone().unwrap_or_default())
                .y_desc(chart.ylabel.clone().unwrap_or_default())
                .draw()?;
            // Grouped bars: split each unit-wide category slot among the series.
            let s = chart.series.len().max(1);
            let slot = 0.8 / s as f64;
            for (si, series) in chart.series.iter().enumerate() {
                let col = color(PALETTE[si % PALETTE.len()]).filled();
                let left0 = -0.4 + si as f64 * slot;
                cc.draw_series(series.y.iter().enumerate().map(|(i, &v)| {
                    let x0 = i as f64 + left0;
                    Rectangle::new([(x0, 0.0), (x0 + slot * 0.92, v)], col)
                }))?;
            }
        }

        ChartKind::Line => {
            let n = longest_series(chart);
            let ymax = chart.y_max() * 1.1;
            let labels = chart.x.clone();
            let mut cc = ChartBuilder::on(&root)
                .caption(caption, title_font())
                .margin(margin)
                .x_label_area_size(x_area)
                .y_label_area_size(y_area)
                .build_cartesian_2d(0f64..(n.saturating_sub(1).max(1) as f64), 0f64..ymax)?;
            cc.configure_mesh()
                .axis_style(axis)
                .light_line_style(grid.mix(0.3))
                .bold_line_style(grid.mix(0.55))
                .label_style(label_font())
                .x_labels(n.max(1))
                .x_label_formatter(&|v: &f64| label_at(&labels, *v))
                .x_desc(chart.xlabel.clone().unwrap_or_default())
                .y_desc(chart.ylabel.clone().unwrap_or_default())
                .draw()?;
            let stroke = ((st.base_px * 0.08) as u32).max(2);
            for (si, series) in chart.series.iter().enumerate() {
                let col = color(PALETTE[si % PALETTE.len()]);
                cc.draw_series(LineSeries::new(
                    series.y.iter().enumerate().map(|(i, &v)| (i as f64, v)),
                    col.stroke_width(stroke),
                ))?
                .label(series.name.clone())
                .legend(move |(x, y)| {
                    PathElement::new(vec![(x, y), (x + 18, y)], col.stroke_width(3))
                });
            }
            if chart.series.len() > 1 {
                cc.configure_series_labels()
                    .border_style(axis)
                    .background_style(color(st.bg).mix(0.85))
                    .label_font(label_font())
                    .draw()?;
            }
        }
    }

    root.present()?;
    Ok(())
}

/// Map an x-axis tick value to its category label, blank for non-integer ticks.
fn label_at(labels: &[String], v: f64) -> String {
    let i = v.round();
    if (v - i).abs() < 1e-6 && i >= 0.0 && (i as usize) < labels.len() {
        labels[i as usize].clone()
    } else {
        String::new()
    }
}

fn draw_pie<DB>(
    chart: &Chart,
    st: &DocStyle,
    root: &DrawingArea<DB, plotters::coord::Shift>,
    ink: RGBColor,
    title_px: i32,
) -> Result<(), Box<dyn std::error::Error>>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let (w, h) = root.dim_in_pixel();
    if let Some(title) = &chart.title {
        root.draw(&Text::new(
            title.clone(),
            (10, 6),
            ("sans-serif", title_px).into_font().color(&ink),
        ))?;
    }
    let center = (w as i32 / 2, h as i32 / 2 + title_px / 2);
    let radius = (w.min(h) as f64 * 0.32).max(8.0);

    let sizes: Vec<f64> = chart.data.iter().map(|(_, v)| *v).collect();
    let labels: Vec<String> = chart
        .data
        .iter()
        .map(|(k, v)| format!("{k} ({})", fmt_num(*v)))
        .collect();
    let colors: Vec<RGBColor> =
        (0..chart.data.len()).map(|i| color(PALETTE[i % PALETTE.len()])).collect();

    let mut pie = Pie::new(&center, &radius, &sizes, &colors, &labels);
    pie.start_angle(-90.0);
    pie.label_style(("sans-serif", (st.base_px * 0.55) as i32).into_font().color(&ink));
    root.draw(&pie)?;
    Ok(())
}

fn longest_series(chart: &Chart) -> usize {
    chart.series.iter().map(|s| s.y.len()).max().unwrap_or(0).max(1)
}

fn scatter_ranges(chart: &Chart) -> (std::ops::Range<f64>, std::ops::Range<f64>) {
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in &chart.points {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    let xpad = ((xmax - xmin) * 0.08).max(0.5);
    let ypad = ((ymax - ymin) * 0.08).max(0.5);
    ((xmin - xpad)..(xmax + xpad), (ymin - ypad)..(ymax + ypad))
}

/// Lift plotters' RGB output to an RGBA image (fully opaque).
fn rgb_to_rgba(rgb: &[u8], width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::new(width, height);
    for (i, px) in img.pixels_mut().enumerate() {
        let o = i * 3;
        *px = Rgba([rgb[o], rgb[o + 1], rgb[o + 2], 255]);
    }
    img
}
