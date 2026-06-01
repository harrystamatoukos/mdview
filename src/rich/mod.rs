//! Rich read-only reader mode (`--rich`).
//!
//! Renders the document with a proportional desktop-class font, rasterized to a
//! bitmap and displayed in the terminal via a graphics protocol. This is a
//! reading experience (no editing), aimed at GUI-app-quality typography.
//!
//! Gated behind the `rich` cargo feature so the default build stays lean.

mod chart_render;
mod clipboard;
mod overlay;
mod paint;
mod view;

use anyhow::Result;
use std::path::{Path, PathBuf};

pub use paint::{CaretMotion, DocStyle, Painter, RichDoc};
pub use view::run;

/// Lay out the document once (shaping + positioning, no rasterization).
/// The painter is returned alongside so the caller can rasterize windows
/// on demand while scrolling.
pub fn lay_out_document(
    markdown: &str,
    style: &DocStyle,
    base_dir: Option<PathBuf>,
) -> (Painter, RichDoc) {
    let elements = crate::parser::parse(markdown);
    let mut painter = Painter::new();
    painter.set_base_dir(base_dir);
    let doc = painter.layout(&elements, style);
    (painter, doc)
}

/// Render the markdown source to a PNG file — verification / preview path.
///
/// Rasterizes the full document in one shot (only used by `--export-png`, which
/// is fine for previews; the interactive reader renders bounded windows).
pub fn export_png(markdown: &str, out: &Path, base_dir: Option<PathBuf>) -> Result<()> {
    let style = DocStyle::light();
    let (mut painter, mut doc) = lay_out_document(markdown, &style, base_dir);
    // Whole document into one image, downscaled to a bounded pixel budget and
    // rendered in tiles so even very long documents don't exhaust memory.
    let (img, _scale) = painter.render_scaled(&mut doc, 8_000_000);
    img.save(out)?;
    Ok(())
}
