//! Read-only "rich" reader: displays the document inline via a terminal
//! graphics protocol and scrolls it smoothly.
//!
//! The whole document is laid out, rasterized **once** (downscaled to a bounded
//! pixel budget, rendered in tiles so memory stays bounded), and transmitted to
//! the terminal once as a `SlicedProtocol`. Scrolling then only moves a cell
//! offset (`SlicedImage`) — nothing is ever re-encoded or re-transmitted, so
//! scrolling is smooth regardless of document length.

use anyhow::{anyhow, Result};
use std::io::stdout;
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
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

use super::DocStyle;

const EVENT_POLL_MS: u64 = 100;
const MAX_EVENTS_PER_FRAME: usize = 64;
/// Pixel budget for the transmitted image. Raw RGBA, so bytes ≈ pixels × 4.
/// Sized so typical documents render at full resolution (no downscale →
/// comfortable text size); only very long documents get scaled down to keep
/// the one-time transmission bounded.
const MAX_PIXELS: u64 = 8_000_000;

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
pub fn run(markdown: &str) -> Result<()> {
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

    let result = run_loop(&mut terminal, &picker, markdown);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    picker: &Picker,
    markdown: &str,
) -> Result<()> {
    let style = DocStyle::light();
    let bg = Color::Rgb(style.bg.0, style.bg.1, style.bg.2);

    // Lay out + rasterize the whole document once, downscaled to the budget.
    let t0 = std::time::Instant::now();
    let (mut painter, mut doc) = super::lay_out_document(markdown, &style);
    perf_log(&format!(
        "layout: {} ms (page_w={}, total_h={})",
        t0.elapsed().as_millis(),
        doc.page_w,
        doc.total_h
    ));

    let t1 = std::time::Instant::now();
    let (img, q) = painter.render_scaled(&mut doc, MAX_PIXELS);
    let (iw, ih) = (img.width(), img.height());
    perf_log(&format!(
        "render_scaled: {} ms (q={q:.3}, img={iw}x{ih} = {} px)",
        t1.elapsed().as_millis(),
        iw as u64 * ih as u64
    ));

    let fs = picker.font_size();
    let cell_w = fs.width.max(1) as u32;
    let cell_h = fs.height.max(1) as u32;

    // Transmit ONCE. Cell dimensions match the image's own pixel size so the
    // protocol doesn't rescale it.
    let cols = iw.div_ceil(cell_w).max(1) as u16;
    let rows = ih.div_ceil(cell_h).max(1) as u16;
    let t2 = std::time::Instant::now();
    let proto = SlicedProtocol::new(
        picker,
        DynamicImage::ImageRgba8(img),
        Some(Size::new(cols, rows)),
    )?;
    let total_rows = proto.size().height;
    perf_log(&format!(
        "protocol build (encode): {} ms (cell={cell_w}x{cell_h}, cols={cols}, rows={rows}, total_rows={total_rows})",
        t2.elapsed().as_millis()
    ));

    let mut frame: u64 = 0;

    let mut scroll_rows: u16 = 0;
    let mut dirty = true;

    loop {
        let term = terminal.size()?;
        let (term_w, term_h) = (term.width, term.height);
        let view_h = term_h.saturating_sub(1).max(1); // reserve status row
        if term_w == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        let max_scroll = total_rows.saturating_sub(view_h);
        if scroll_rows > max_scroll {
            scroll_rows = max_scroll;
        }

        if dirty {
            let x_off = (term_w.saturating_sub(cols)) / 2;
            let img_area = Rect::new(x_off, 0, cols.min(term_w), view_h);
            let full = Rect::new(0, 0, term_w, view_h);
            let position = SignedPosition::from((0, -(scroll_rows as i16)));

            let pct = if max_scroll == 0 {
                100
            } else {
                (scroll_rows as f32 / max_scroll as f32 * 100.0).round() as u32
            };
            let status = format!(
                " mdview · reading · {pct}%    ↑/↓ j/k scroll · space page · g/G top/bottom · q quit "
            );
            let status_rect = Rect::new(0, term_h.saturating_sub(1), term_w, 1);

            let td = std::time::Instant::now();
            terminal.draw(|f| {
                f.render_widget(Block::default().style(Style::default().bg(bg)), full);
                f.render_widget(SlicedImage::new(&proto, position), img_area);
                f.render_widget(
                    Paragraph::new(status).style(Style::default().fg(Color::Rgb(120, 120, 120)).bg(bg)),
                    status_rect,
                );
            })?;
            frame += 1;
            perf_log(&format!(
                "draw #{frame}: {} ms (scroll_rows={scroll_rows})",
                td.elapsed().as_millis()
            ));
            dirty = false;
        }

        let page = view_h.saturating_sub(2).max(1);

        if event::poll(Duration::from_millis(EVENT_POLL_MS))? {
            for _ in 0..MAX_EVENTS_PER_FRAME {
                match handle_event(event::read()?, &mut scroll_rows, max_scroll, page) {
                    RichEvent::Quit => return Ok(()),
                    RichEvent::Changed => dirty = true,
                    RichEvent::Ignored => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

fn handle_event(input: Event, scroll: &mut u16, max_scroll: u16, page: u16) -> RichEvent {
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
