use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    text::Text,
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use std::io::{self, stdout};

use crate::renderer;
use crate::theme::{Theme, LEFT_MARGIN};

/// Smooth scrolling pager with mouse support
pub struct Pager<'a> {
    content: Text<'a>,
    scroll_target: f64,    // Where we want to scroll to
    scroll_current: f64,   // Animated current position
    total_lines: usize,
    theme: Theme,
}

// Easing factor: higher = snappier, lower = smoother
const SCROLL_EASING: f64 = 0.25;
// Threshold to snap to target (avoid endless micro-animations)
const SCROLL_SNAP_THRESHOLD: f64 = 0.5;

impl<'a> Pager<'a> {
    pub fn new(content: Text<'a>, theme: Theme) -> Self {
        let total_lines = content.lines.len();
        Self {
            content,
            scroll_target: 0.0,
            scroll_current: 0.0,
            total_lines,
            theme,
        }
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll_target = (self.scroll_target - amount as f64).max(0.0);
    }

    fn scroll_down(&mut self, amount: usize, viewport_height: usize) {
        let max_scroll = self.total_lines.saturating_sub(viewport_height) as f64;
        self.scroll_target = (self.scroll_target + amount as f64).min(max_scroll);
    }

    fn scroll_to_top(&mut self) {
        self.scroll_target = 0.0;
    }

    fn scroll_to_bottom(&mut self, viewport_height: usize) {
        self.scroll_target = self.total_lines.saturating_sub(viewport_height) as f64;
    }

    /// Update animation state, returns true if still animating
    fn update_animation(&mut self) -> bool {
        let diff = self.scroll_target - self.scroll_current;

        if diff.abs() < SCROLL_SNAP_THRESHOLD {
            // Snap to target when close enough
            self.scroll_current = self.scroll_target;
            false
        } else {
            // Exponential easing toward target
            self.scroll_current += diff * SCROLL_EASING;
            true
        }
    }

    /// Get the current scroll position for rendering
    fn scroll_position(&self) -> usize {
        self.scroll_current.round() as usize
    }
}

/// Run the interactive pager
pub fn run(content: &str, theme: Theme) -> Result<()> {
    // Setup terminal with mouse support
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Get terminal size and render content
    let size = terminal.size()?;
    let text = renderer::render_to_text(content, size.width.saturating_sub(LEFT_MARGIN as u16), &theme);
    let mut pager = Pager::new(text, theme);

    let result = run_event_loop(&mut terminal, &mut pager);

    // Cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pager: &mut Pager,
) -> Result<()> {
    loop {
        let viewport_height = terminal.size()?.height.saturating_sub(2) as usize;

        // Update animation
        let animating = pager.update_animation();

        terminal.draw(|frame| draw(frame, pager, viewport_height))?;

        // Use shorter poll timeout when animating for smoother animation
        let poll_timeout = if animating {
            std::time::Duration::from_millis(16) // ~60fps
        } else {
            std::time::Duration::from_millis(50) // Idle, save CPU
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        // Quit
                        KeyCode::Char('q') | KeyCode::Esc => break,

                        // Scroll up
                        KeyCode::Up | KeyCode::Char('k') => pager.scroll_up(1),
                        KeyCode::Char('u') => pager.scroll_up(viewport_height / 2),
                        KeyCode::PageUp | KeyCode::Char('b') => pager.scroll_up(viewport_height),

                        // Scroll down
                        KeyCode::Down | KeyCode::Char('j') => {
                            pager.scroll_down(1, viewport_height)
                        }
                        KeyCode::Char('d') => pager.scroll_down(viewport_height / 2, viewport_height),
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            pager.scroll_down(viewport_height, viewport_height)
                        }

                        // Jump to top/bottom
                        KeyCode::Char('g') => pager.scroll_to_top(),
                        KeyCode::Char('G') => pager.scroll_to_bottom(viewport_height),
                        KeyCode::Home => pager.scroll_to_top(),
                        KeyCode::End => pager.scroll_to_bottom(viewport_height),

                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => pager.scroll_up(3),
                        MouseEventKind::ScrollDown => pager.scroll_down(3, viewport_height),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn draw(frame: &mut Frame, pager: &Pager, viewport_height: usize) {
    let area = frame.area();

    // Layout: main content + scrollbar
    let chunks = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    // Content area with padding
    let content_area = Rect {
        x: chunks[0].x + (LEFT_MARGIN / 2) as u16,
        y: chunks[0].y + 1,
        width: chunks[0].width.saturating_sub(LEFT_MARGIN as u16),
        height: chunks[0].height.saturating_sub(2),
    };

    let scroll_pos = pager.scroll_position();

    // Render paragraph with scroll (using reference, not clone)
    let paragraph = Paragraph::new(pager.content.clone())
        .scroll((scroll_pos as u16, 0));

    frame.render_widget(paragraph, content_area);

    // Scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(pager.total_lines)
        .position(scroll_pos)
        .viewport_content_length(viewport_height);

    frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);

    // Status bar at bottom
    let status = format!(
        " mdview │ {}-{} of {} │ q:quit  j/k:scroll  g/G:top/bottom ",
        scroll_pos + 1,
        (scroll_pos + viewport_height).min(pager.total_lines),
        pager.total_lines
    );

    let status_area = Rect {
        x: area.x,
        y: area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    let status_bar = Paragraph::new(status).style(pager.theme.status_bar());

    frame.render_widget(status_bar, status_area);
}
