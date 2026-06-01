use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Text},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use std::io::stdout;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::renderer;
use crate::theme::Theme;
use crate::view::ViewState;

const ACTIVE_POLL_MS: u64 = 16;
const IDLE_POLL_MS: u64 = 50;
const MAX_EVENTS_PER_FRAME: usize = 32;

enum WatchEvent {
    Quit,
    Changed,
    Ignored,
}

struct WatchPager {
    content: Text<'static>,
    view: ViewState,
    total_lines: usize,
    last_refresh: Instant,
    show_refresh_indicator: bool,
    theme: Theme,
}

impl WatchPager {
    fn new(content: Text<'static>, theme: Theme) -> Self {
        let total_lines = content.lines.len();
        Self {
            content,
            view: ViewState::new(),
            total_lines,
            last_refresh: Instant::now(),
            show_refresh_indicator: false,
            theme,
        }
    }

    fn update_content(&mut self, content: Text<'static>, viewport_height: usize) {
        self.total_lines = content.lines.len();
        self.content = content;
        self.view
            .clamp_to_content(self.total_lines, viewport_height);
        self.last_refresh = Instant::now();
        self.show_refresh_indicator = true;
    }

    fn scroll_up(&mut self, amount: usize) {
        self.view.scroll_up(amount);
    }

    fn scroll_down(&mut self, amount: usize, viewport_height: usize) {
        self.view
            .scroll_down(amount, self.total_lines, viewport_height);
    }

    fn scroll_to_top(&mut self) {
        self.view.scroll_to_top();
    }

    fn scroll_to_bottom(&mut self, viewport_height: usize) {
        self.view
            .scroll_to_bottom(self.total_lines, viewport_height);
    }

    /// Update animation state, returns true if still animating
    fn update_animation(&mut self) -> bool {
        self.view.update_animation()
    }

    fn scroll_position(&self) -> usize {
        self.view.position()
    }

    fn tick(&mut self) -> bool {
        if self.show_refresh_indicator && self.last_refresh.elapsed() > Duration::from_secs(2) {
            self.show_refresh_indicator = false;
            true
        } else {
            false
        }
    }
}

/// Watch a file and display with auto-refresh
pub fn watch_and_display(path: &Path, theme: Theme) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let content = std::fs::read_to_string(path)?;
    let text = renderer::render_to_text(&content, size.width.saturating_sub(4), &theme);
    let mut pager = WatchPager::new(text, theme);

    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            if let Ok(_event) = res {
                let _ = tx.send(());
            }
        },
        Config::default(),
    )?;

    watcher.watch(path, RecursiveMode::NonRecursive)?;

    let result = run_watch_loop(&mut terminal, &mut pager, &rx, path);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    result
}

fn run_watch_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    pager: &mut WatchPager,
    file_rx: &mpsc::Receiver<()>,
    path: &Path,
) -> Result<()> {
    let mut dirty = true;

    loop {
        let viewport_height = terminal.size()?.height.saturating_sub(2) as usize;
        let width = terminal.size()?.width;

        // Check for file changes
        if file_rx.try_recv().is_ok() {
            // Drain any queued events
            while file_rx.try_recv().is_ok() {}

            if let Ok(content) = std::fs::read_to_string(path) {
                let text =
                    renderer::render_to_text(&content, width.saturating_sub(4), &pager.theme);
                pager.update_content(text, viewport_height);
                dirty = true;
            }
        }

        dirty |= pager.tick();

        // Update animation
        let animating = pager.update_animation();
        dirty |= animating;

        if dirty {
            terminal.draw(|frame| draw_watch(frame, pager, viewport_height, path))?;
            dirty = false;
        }

        // Use shorter poll timeout when animating for smoother animation
        let poll_timeout = if animating {
            Duration::from_millis(ACTIVE_POLL_MS) // ~60fps
        } else {
            Duration::from_millis(IDLE_POLL_MS) // Idle, save CPU
        };

        if event::poll(poll_timeout)? {
            for _ in 0..MAX_EVENTS_PER_FRAME {
                match handle_watch_event(event::read()?, pager, viewport_height, width, path) {
                    WatchEvent::Quit => return Ok(()),
                    WatchEvent::Changed => dirty = true,
                    WatchEvent::Ignored => {}
                }

                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

fn handle_watch_event(
    input_event: Event,
    pager: &mut WatchPager,
    viewport_height: usize,
    width: u16,
    path: &Path,
) -> WatchEvent {
    match input_event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => WatchEvent::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                pager.scroll_up(1);
                WatchEvent::Changed
            }
            KeyCode::Char('u') => {
                pager.scroll_up(viewport_height / 2);
                WatchEvent::Changed
            }
            KeyCode::PageUp | KeyCode::Char('b') => {
                pager.scroll_up(viewport_height);
                WatchEvent::Changed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                pager.scroll_down(1, viewport_height);
                WatchEvent::Changed
            }
            KeyCode::Char('d') => {
                pager.scroll_down(viewport_height / 2, viewport_height);
                WatchEvent::Changed
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                pager.scroll_down(viewport_height, viewport_height);
                WatchEvent::Changed
            }
            KeyCode::Char('g') => {
                pager.scroll_to_top();
                WatchEvent::Changed
            }
            KeyCode::Char('G') => {
                pager.scroll_to_bottom(viewport_height);
                WatchEvent::Changed
            }
            KeyCode::Home => {
                pager.scroll_to_top();
                WatchEvent::Changed
            }
            KeyCode::End => {
                pager.scroll_to_bottom(viewport_height);
                WatchEvent::Changed
            }
            KeyCode::Char('r') => {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let text =
                        renderer::render_to_text(&content, width.saturating_sub(4), &pager.theme);
                    pager.update_content(text, viewport_height);
                    WatchEvent::Changed
                } else {
                    WatchEvent::Ignored
                }
            }
            _ => WatchEvent::Ignored,
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                pager.scroll_up(3);
                WatchEvent::Changed
            }
            MouseEventKind::ScrollDown => {
                pager.scroll_down(3, viewport_height);
                WatchEvent::Changed
            }
            _ => WatchEvent::Ignored,
        },
        Event::Resize(_, _) => WatchEvent::Changed,
        _ => WatchEvent::Ignored,
    }
}

fn draw_watch(frame: &mut Frame, pager: &WatchPager, viewport_height: usize, path: &Path) {
    let area = frame.area();

    // Paint the page color across the whole screen (see pager::draw).
    frame.buffer_mut().set_style(area, pager.theme.canvas());

    let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(area);

    let content_area = Rect {
        x: chunks[0].x + 2,
        y: chunks[0].y + 1,
        width: chunks[0].width.saturating_sub(4),
        height: chunks[0].height.saturating_sub(2),
    };

    let scroll_pos = pager.scroll_position();

    let total = pager.content.lines.len();
    let start = scroll_pos.min(total);
    let end = (scroll_pos + content_area.height as usize).min(total);
    render_visible_lines(
        frame,
        &pager.content.lines[start..end],
        content_area,
        pager.theme.canvas(),
    );

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(pager.total_lines)
        .position(scroll_pos)
        .viewport_content_length(viewport_height);

    frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);

    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    let refresh_indicator = if pager.show_refresh_indicator {
        " ● "
    } else {
        ""
    };

    let status = format!(
        " mdview │ {} │ watching{} │ {}-{} of {} │ q:quit  r:refresh ",
        filename,
        refresh_indicator,
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

    let status_style = if pager.show_refresh_indicator {
        pager.theme.status_bar_accent()
    } else {
        pager.theme.status_bar()
    };

    let status_bar = Paragraph::new(status).style(status_style);

    frame.render_widget(status_bar, status_area);
}

fn render_visible_lines(frame: &mut Frame, lines: &[Line<'static>], area: Rect, style: Style) {
    let buf = frame.buffer_mut();
    buf.set_style(area, style);
    for (row, line) in lines.iter().take(area.height as usize).enumerate() {
        buf.set_line(area.x, area.y + row as u16, line, area.width);
    }
}
