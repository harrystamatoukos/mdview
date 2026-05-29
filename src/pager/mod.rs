//! Read-only terminal reader (classic `--tui` view).
//!
//! Renders the document to styled terminal text once and scrolls it smoothly.
//! No editing, cursor, or selection — this is purely a reader.

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use std::io::{self, stdout};

use crate::renderer;
use crate::theme::Theme;
use crate::view::ViewState;

// Poll cadence: fast while animating for smooth scrolling, idle otherwise.
const ACTIVE_POLL_MS: u64 = 16;
const IDLE_POLL_MS: u64 = 50;
const MAX_EVENTS_PER_FRAME: usize = 32;

enum ReaderEvent {
    Quit,
    Changed,
    Ignored,
}

struct Reader {
    content: String,
    theme: Theme,
    text: Text<'static>,
    total_lines: usize,
    view: ViewState,
    width: u16,
}

impl Reader {
    fn new(content: String, theme: Theme, width: u16) -> Self {
        let text = renderer::render_to_text(&content, width, &theme);
        let total_lines = text.lines.len();
        Self {
            content,
            theme,
            text,
            total_lines,
            view: ViewState::new(),
            width,
        }
    }

    /// Re-render when the terminal width changes (affects wrapping/centering).
    fn ensure_width(&mut self, width: u16, viewport_height: usize) -> bool {
        if width != self.width {
            self.width = width;
            self.text = renderer::render_to_text(&self.content, width, &self.theme);
            self.total_lines = self.text.lines.len();
            self.view.clamp_to_content(self.total_lines, viewport_height);
            true
        } else {
            false
        }
    }

    fn scroll_position(&self) -> usize {
        self.view.position()
    }
}

/// Run the read-only reader for the given markdown source.
pub fn run(content: &str, theme: Theme) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let width = terminal.size()?.width;
    let mut reader = Reader::new(content.to_string(), theme, width);

    let result = run_loop(&mut terminal, &mut reader);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    reader: &mut Reader,
) -> Result<()> {
    let mut dirty = true;

    loop {
        let size = terminal.size()?;
        let viewport_height = size.height.saturating_sub(2) as usize;
        if reader.ensure_width(size.width, viewport_height) {
            dirty = true;
        }

        let animating = reader.view.update_animation();
        dirty |= animating;

        if dirty {
            terminal.draw(|frame| draw(frame, reader, viewport_height))?;
            dirty = false;
        }

        let poll_timeout = if animating {
            std::time::Duration::from_millis(ACTIVE_POLL_MS)
        } else {
            std::time::Duration::from_millis(IDLE_POLL_MS)
        };

        if event::poll(poll_timeout)? {
            for _ in 0..MAX_EVENTS_PER_FRAME {
                match handle_event(event::read()?, reader, viewport_height) {
                    ReaderEvent::Quit => return Ok(()),
                    ReaderEvent::Changed => dirty = true,
                    ReaderEvent::Ignored => {}
                }
                if !event::poll(std::time::Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

fn handle_event(input: Event, reader: &mut Reader, viewport_height: usize) -> ReaderEvent {
    match input {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => ReaderEvent::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                reader.view.scroll_up(1);
                ReaderEvent::Changed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                reader.view.scroll_down(1, reader.total_lines, viewport_height);
                ReaderEvent::Changed
            }
            KeyCode::Char('u') => {
                reader.view.scroll_up(viewport_height / 2);
                ReaderEvent::Changed
            }
            KeyCode::Char('d') => {
                reader.view.scroll_down(viewport_height / 2, reader.total_lines, viewport_height);
                ReaderEvent::Changed
            }
            KeyCode::PageUp | KeyCode::Char('b') => {
                reader.view.scroll_up(viewport_height);
                ReaderEvent::Changed
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                reader.view.scroll_down(viewport_height, reader.total_lines, viewport_height);
                ReaderEvent::Changed
            }
            KeyCode::Char('g') | KeyCode::Home => {
                reader.view.scroll_to_top();
                ReaderEvent::Changed
            }
            KeyCode::Char('G') | KeyCode::End => {
                reader.view.scroll_to_bottom(reader.total_lines, viewport_height);
                ReaderEvent::Changed
            }
            _ => ReaderEvent::Ignored,
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                reader.view.scroll_up(3);
                ReaderEvent::Changed
            }
            MouseEventKind::ScrollDown => {
                reader.view.scroll_down(3, reader.total_lines, viewport_height);
                ReaderEvent::Changed
            }
            _ => ReaderEvent::Ignored,
        },
        Event::Resize(_, _) => ReaderEvent::Changed,
        _ => ReaderEvent::Ignored,
    }
}

fn draw(frame: &mut Frame, reader: &Reader, viewport_height: usize) {
    let area = frame.area();

    // Paint the page color across the whole screen.
    frame.buffer_mut().set_style(area, reader.theme.canvas());

    let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(area);

    let content_area = Rect {
        x: chunks[0].x,
        y: chunks[0].y + 1,
        width: chunks[0].width,
        height: chunks[0].height.saturating_sub(2),
    };

    let scroll_pos = reader.scroll_position();

    let total = reader.text.lines.len();
    let start = scroll_pos.min(total);
    let end = (scroll_pos + content_area.height as usize).min(total);
    render_visible_lines(frame, &reader.text.lines[start..end], content_area, reader.theme.canvas());

    // Scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(reader.total_lines)
        .position(scroll_pos)
        .viewport_content_length(viewport_height);
    frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);

    // Status bar
    let pct = if reader.total_lines > viewport_height {
        (scroll_pos as f32 / (reader.total_lines - viewport_height) as f32 * 100.0)
            .round()
            .clamp(0.0, 100.0) as u32
    } else {
        100
    };
    let status = format!(
        " mdview · reading · {pct}%    ↑/↓ j/k scroll · space page · g/G top/bottom · q quit "
    );
    let status_rect = Rect {
        x: area.x,
        y: area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let status_bar = Paragraph::new(status).style(reader.theme.status_bar().add_modifier(Modifier::DIM));
    frame.render_widget(status_bar, status_rect);
}

fn render_visible_lines(frame: &mut Frame, lines: &[Line<'static>], area: Rect, style: Style) {
    let buf = frame.buffer_mut();
    buf.set_style(area, style);
    for (row, line) in lines.iter().take(area.height as usize).enumerate() {
        buf.set_line(area.x, area.y + row as u16, line, area.width);
    }
}
