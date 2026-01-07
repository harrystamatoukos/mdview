use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    text::Text,
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use std::io::stdout;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::renderer;
use crate::theme::Theme;

// Easing factor: higher = snappier, lower = smoother
const SCROLL_EASING: f64 = 0.25;
// Threshold to snap to target (avoid endless micro-animations)
const SCROLL_SNAP_THRESHOLD: f64 = 0.5;

struct WatchPager {
    content: Text<'static>,
    scroll_target: f64,
    scroll_current: f64,
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
            scroll_target: 0.0,
            scroll_current: 0.0,
            total_lines,
            last_refresh: Instant::now(),
            show_refresh_indicator: false,
            theme,
        }
    }

    fn update_content(&mut self, content: Text<'static>) {
        self.total_lines = content.lines.len();
        self.content = content;
        // Clamp scroll to valid range
        let max_scroll = self.total_lines.saturating_sub(1) as f64;
        self.scroll_target = self.scroll_target.min(max_scroll);
        self.scroll_current = self.scroll_current.min(max_scroll);
        self.last_refresh = Instant::now();
        self.show_refresh_indicator = true;
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
            self.scroll_current = self.scroll_target;
            false
        } else {
            self.scroll_current += diff * SCROLL_EASING;
            true
        }
    }

    fn scroll_position(&self) -> usize {
        self.scroll_current.round() as usize
    }

    fn tick(&mut self) {
        if self.show_refresh_indicator && self.last_refresh.elapsed() > Duration::from_secs(2) {
            self.show_refresh_indicator = false;
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
    let (text, _) = renderer::render_to_text(&content, size.width.saturating_sub(4), &theme);
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

    result
}

fn run_watch_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    pager: &mut WatchPager,
    file_rx: &mpsc::Receiver<()>,
    path: &Path,
) -> Result<()> {
    loop {
        let viewport_height = terminal.size()?.height.saturating_sub(2) as usize;
        let width = terminal.size()?.width;

        // Check for file changes
        if file_rx.try_recv().is_ok() {
            // Drain any queued events
            while file_rx.try_recv().is_ok() {}

            if let Ok(content) = std::fs::read_to_string(path) {
                let (text, _) = renderer::render_to_text(&content, width.saturating_sub(4), &pager.theme);
                pager.update_content(text);
            }
        }

        pager.tick();

        // Update animation
        let animating = pager.update_animation();

        terminal.draw(|frame| draw_watch(frame, pager, viewport_height, path))?;

        // Use shorter poll timeout when animating for smoother animation
        let poll_timeout = if animating {
            Duration::from_millis(16) // ~60fps
        } else {
            Duration::from_millis(50) // Idle, save CPU
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up | KeyCode::Char('k') => pager.scroll_up(1),
                        KeyCode::Char('u') => pager.scroll_up(viewport_height / 2),
                        KeyCode::PageUp | KeyCode::Char('b') => pager.scroll_up(viewport_height),
                        KeyCode::Down | KeyCode::Char('j') => {
                            pager.scroll_down(1, viewport_height)
                        }
                        KeyCode::Char('d') => pager.scroll_down(viewport_height / 2, viewport_height),
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            pager.scroll_down(viewport_height, viewport_height)
                        }
                        KeyCode::Char('g') => pager.scroll_to_top(),
                        KeyCode::Char('G') => pager.scroll_to_bottom(viewport_height),
                        KeyCode::Home => pager.scroll_to_top(),
                        KeyCode::End => pager.scroll_to_bottom(viewport_height),
                        KeyCode::Char('r') => {
                            if let Ok(content) = std::fs::read_to_string(path) {
                                let (text, _) = renderer::render_to_text(&content, width.saturating_sub(4), &pager.theme);
                                pager.update_content(text);
                            }
                        }
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

fn draw_watch(frame: &mut Frame, pager: &WatchPager, viewport_height: usize, path: &Path) {
    let area = frame.area();

    let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(area);

    let content_area = Rect {
        x: chunks[0].x + 2,
        y: chunks[0].y + 1,
        width: chunks[0].width.saturating_sub(4),
        height: chunks[0].height.saturating_sub(2),
    };

    let scroll_pos = pager.scroll_position();

    let paragraph = Paragraph::new(pager.content.clone()).scroll((scroll_pos as u16, 0));

    frame.render_widget(paragraph, content_area);

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
