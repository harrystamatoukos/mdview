use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::Text,
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use std::io::{self, stdout};

use crate::editor::EditorState;
use crate::position::LayoutMap;
use crate::renderer;
use crate::selection::Selection;
use crate::theme::{Theme, LEFT_MARGIN};

/// Smooth scrolling pager with cursor, mouse support, and editing
pub struct Pager<'a> {
    // Rendered content
    content: Text<'a>,
    layout_map: LayoutMap,
    scroll_target: f64,    // Where we want to scroll to
    scroll_current: f64,   // Animated current position
    total_lines: usize,
    theme: Theme,
    // Cursor state for WYSIWYG editing
    cursor_source: Option<usize>,  // Source byte offset (None = no cursor)
    cursor_visible: bool,
    // Selection state
    selection: Selection,
    // Editor state for text manipulation
    editor: EditorState,
    terminal_width: u16,
    // Edit mode flag
    edit_mode: bool,
    // Content area for mouse click handling (x, y, width, height)
    content_area: (u16, u16, u16, u16),
    // Mouse drag state for selection
    mouse_dragging: bool,
}

// Easing factor: higher = snappier, lower = smoother
const SCROLL_EASING: f64 = 0.25;
// Threshold to snap to target (avoid endless micro-animations)
const SCROLL_SNAP_THRESHOLD: f64 = 0.5;

impl<'a> Pager<'a> {
    pub fn new(editor: EditorState, terminal_width: u16, theme: Theme) -> Self {
        // Initial render
        let (content, layout_map) = renderer::render_to_text_mapped(
            editor.content(),
            terminal_width.saturating_sub(LEFT_MARGIN as u16),
            &theme
        );
        let total_lines = content.lines.len();
        // Start cursor at first valid source position
        let cursor_source = layout_map.first_offset();

        // Initialize selection at cursor position
        let selection = Selection::new(cursor_source.unwrap_or(0));

        Self {
            content,
            layout_map,
            scroll_target: 0.0,
            scroll_current: 0.0,
            total_lines,
            theme,
            cursor_source,
            cursor_visible: true,
            selection,
            editor,
            terminal_width,
            edit_mode: true, // Start in edit mode
            content_area: (0, 0, 0, 0), // Will be set on first draw
            mouse_dragging: false,
        }
    }

    /// Update content area dimensions (called from draw)
    fn set_content_area(&mut self, x: u16, y: u16, width: u16, height: u16) {
        self.content_area = (x, y, width, height);
    }

    /// Re-render content after editing
    /// Preserves cursor position where possible
    fn re_render(&mut self) {
        // Re-render with current editor content
        let (content, layout_map) = renderer::render_to_text_mapped(
            self.editor.content(),
            self.terminal_width.saturating_sub(LEFT_MARGIN as u16),
            &self.theme
        );

        self.content = content;
        self.layout_map = layout_map;
        self.total_lines = self.content.lines.len();

        // Sync cursor from editor state
        let editor_cursor = self.editor.cursor();
        self.cursor_source = Some(editor_cursor);

        // Build index for fast lookups
        self.layout_map.build_index();
    }

    /// Handle a character input for editing
    fn handle_char_input(&mut self, ch: char) {
        if !self.edit_mode {
            return;
        }

        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
        }

        // Sync editor cursor from current visual cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Insert the character
        self.editor.insert_char(ch);
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());

        // Re-render
        self.re_render();
    }

    /// Handle backspace (delete before cursor)
    fn handle_backspace(&mut self) {
        if !self.edit_mode {
            return;
        }

        // If there's a selection, delete it instead
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Delete
        if self.editor.delete_before() {
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
            self.re_render();
        }
    }

    /// Handle delete key (delete at cursor)
    fn handle_delete(&mut self) {
        if !self.edit_mode {
            return;
        }

        // If there's a selection, delete it instead
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Delete
        if self.editor.delete_at() {
            self.cursor_source = Some(self.editor.cursor());
            self.re_render();
        }
    }

    /// Handle enter key (insert newline)
    fn handle_enter(&mut self) {
        if !self.edit_mode {
            return;
        }

        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Insert newline
        self.editor.insert_newline();
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());
        self.re_render();
    }

    /// Toggle edit mode
    fn toggle_edit_mode(&mut self) {
        self.edit_mode = !self.edit_mode;
    }

    /// Check if in edit mode
    fn is_edit_mode(&self) -> bool {
        self.edit_mode
    }

    /// Check if content is dirty
    fn is_dirty(&self) -> bool {
        self.editor.is_dirty()
    }

    /// Check if there's an active selection
    fn has_selection(&self) -> bool {
        self.selection.is_active()
    }

    /// Returns the selected text, if any
    fn selected_text(&self) -> Option<String> {
        if !self.selection.is_active() {
            return None;
        }
        let (start, end) = self.selection.range();
        let content = self.editor.content();
        if end <= content.len() {
            Some(content[start..end].to_string())
        } else {
            None
        }
    }

    /// Delete selected text and collapse selection
    fn delete_selection(&mut self) {
        if !self.selection.is_active() {
            return;
        }

        let (start, end) = self.selection.range();
        self.editor.set_cursor(start);
        self.editor.delete_range(start, end);
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());
        self.re_render();
    }

    /// Copy selected text to clipboard
    fn copy_selection(&self) {
        if let Some(text) = self.selected_text() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(text);
            }
        }
    }

    /// Cut selected text to clipboard
    fn cut_selection(&mut self) {
        self.copy_selection();
        self.delete_selection();
    }

    /// Paste from clipboard at cursor position
    fn paste(&mut self) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Ok(text) = clipboard.get_text() {
                // Delete selection first if any
                if self.selection.is_active() {
                    self.delete_selection();
                }

                // Sync cursor and insert
                if let Some(offset) = self.cursor_source {
                    self.editor.set_cursor(offset);
                }
                self.editor.insert_str(&text);
                self.cursor_source = Some(self.editor.cursor());
                self.selection.move_to(self.editor.cursor());
                self.re_render();
            }
        }
    }

    /// Select all text
    fn select_all(&mut self) {
        self.selection.select_all(self.editor.content().len());
        // Move cursor to end of selection
        self.cursor_source = Some(self.selection.cursor);
    }

    /// Clear selection (collapse to cursor)
    fn clear_selection(&mut self) {
        if let Some(cursor) = self.cursor_source {
            self.selection.move_to(cursor);
        }
    }

    /// Extend selection to a new position (for Shift+Arrow)
    fn extend_selection_to(&mut self, new_cursor: usize) {
        self.selection.extend_to(new_cursor);
        self.cursor_source = Some(new_cursor);
    }

    /// Move cursor and clear selection (for Arrow without Shift)
    fn move_cursor_to(&mut self, new_cursor: usize) {
        self.selection.move_to(new_cursor);
        self.cursor_source = Some(new_cursor);
    }

    /// Handle mouse drag start
    fn start_mouse_drag(&mut self, screen_x: u16, screen_y: u16) {
        let (area_x, area_y, area_width, area_height) = self.content_area;

        // Check if click is within content area
        if screen_x < area_x || screen_x >= area_x + area_width
            || screen_y < area_y || screen_y >= area_y + area_height
        {
            return;
        }

        // Convert screen position to content position
        let content_col = (screen_x - area_x) as usize;
        let scroll_pos = self.scroll_position();
        let content_line = scroll_pos + (screen_y - area_y) as usize;

        // Start selection at clicked position
        if let Some(offset) = self.layout_map.screen_to_source_nearest(content_line, content_col) {
            self.selection = Selection::new(offset);
            self.cursor_source = Some(offset);
            self.editor.set_cursor(offset);
            self.mouse_dragging = true;
        }
    }

    /// Handle mouse drag movement
    fn update_mouse_drag(&mut self, screen_x: u16, screen_y: u16) {
        if !self.mouse_dragging {
            return;
        }

        let (area_x, area_y, area_width, area_height) = self.content_area;

        // Clamp to content area bounds
        let clamped_x = screen_x.max(area_x).min(area_x + area_width - 1);
        let clamped_y = screen_y.max(area_y).min(area_y + area_height - 1);

        let content_col = (clamped_x - area_x) as usize;
        let scroll_pos = self.scroll_position();
        let content_line = scroll_pos + (clamped_y - area_y) as usize;

        // Extend selection to current position
        if let Some(offset) = self.layout_map.screen_to_source_nearest(content_line, content_col) {
            self.selection.extend_to(offset);
            self.cursor_source = Some(offset);
        }
    }

    /// Handle mouse drag end
    fn end_mouse_drag(&mut self) {
        self.mouse_dragging = false;
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

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor navigation (WYSIWYG editing support)
    // ─────────────────────────────────────────────────────────────────────────

    /// Move cursor to next valid position (right arrow)
    fn cursor_right(&mut self) {
        if let Some(current) = self.cursor_source {
            if let Some(next) = self.layout_map.next_cursor_position(current) {
                self.cursor_source = Some(next);
                self.ensure_cursor_visible();
            }
        }
    }

    /// Move cursor to previous valid position (left arrow)
    fn cursor_left(&mut self) {
        if let Some(current) = self.cursor_source {
            if let Some(prev) = self.layout_map.prev_cursor_position(current) {
                self.cursor_source = Some(prev);
                self.ensure_cursor_visible();
            }
        }
    }

    /// Move cursor up one line
    fn cursor_up(&mut self, viewport_height: usize) {
        if let Some(current) = self.cursor_source {
            if let Some((line, col)) = self.layout_map.source_to_screen(current) {
                if line > 0 {
                    // Try to maintain column position on line above
                    if let Some(offset) = self.layout_map.screen_to_source_nearest(line - 1, col) {
                        self.cursor_source = Some(offset);
                        self.ensure_cursor_visible_with_viewport(viewport_height);
                    }
                }
            }
        }
    }

    /// Move cursor down one line
    fn cursor_down(&mut self, viewport_height: usize) {
        if let Some(current) = self.cursor_source {
            if let Some((line, col)) = self.layout_map.source_to_screen(current) {
                if line + 1 < self.layout_map.line_count() {
                    // Try to maintain column position on line below
                    if let Some(offset) = self.layout_map.screen_to_source_nearest(line + 1, col) {
                        self.cursor_source = Some(offset);
                        self.ensure_cursor_visible_with_viewport(viewport_height);
                    }
                }
            }
        }
    }

    /// Ensure cursor is visible by scrolling if needed
    fn ensure_cursor_visible(&mut self) {
        self.ensure_cursor_visible_with_viewport(20); // Default viewport estimate
    }

    fn ensure_cursor_visible_with_viewport(&mut self, viewport_height: usize) {
        if let Some(current) = self.cursor_source {
            if let Some((line, _)) = self.layout_map.source_to_screen(current) {
                let scroll_pos = self.scroll_position();

                // Scroll up if cursor is above viewport
                if line < scroll_pos {
                    self.scroll_target = line as f64;
                }
                // Scroll down if cursor is below viewport
                else if line >= scroll_pos + viewport_height {
                    self.scroll_target = (line - viewport_height + 1) as f64;
                }
            }
        }
    }

    /// Get cursor screen position (line, col) relative to content
    fn cursor_screen_position(&self) -> Option<(usize, usize)> {
        self.cursor_source.and_then(|offset| self.layout_map.source_to_screen(offset))
    }

    /// Get screen positions for active selection (for highlighting)
    fn selection_screen_positions(&self) -> Vec<(usize, usize)> {
        if !self.selection.is_active() {
            return Vec::new();
        }
        let (start, end) = self.selection.range();
        self.layout_map.source_range_to_screen(start, end)
    }

    /// Position cursor from mouse click (screen coordinates)
    /// Returns true if cursor was positioned
    fn position_cursor_from_click(&mut self, screen_x: u16, screen_y: u16, content_area: (u16, u16, u16, u16)) {
        let (area_x, area_y, area_width, area_height) = content_area;

        // Check if click is within content area
        if screen_x < area_x || screen_x >= area_x + area_width
            || screen_y < area_y || screen_y >= area_y + area_height
        {
            return;
        }

        // Convert screen position to content position
        let content_col = (screen_x - area_x) as usize;
        let scroll_pos = self.scroll_position();
        let content_line = scroll_pos + (screen_y - area_y) as usize;

        // Try to find source offset at this position
        if let Some(offset) = self.layout_map.screen_to_source_nearest(content_line, content_col) {
            self.cursor_source = Some(offset);
            // Sync editor cursor
            self.editor.set_cursor(offset);
        }
    }

    /// Toggle cursor visibility
    fn toggle_cursor(&mut self) {
        self.cursor_visible = !self.cursor_visible;
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

    // Get terminal size and create editor state
    let size = terminal.size()?;
    let editor = EditorState::new(content.to_string());
    let mut pager = Pager::new(editor, size.width, theme);

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
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

                    // Handle based on edit mode
                    if pager.is_edit_mode() {
                        match key.code {
                            // Escape exits edit mode (back to view mode)
                            KeyCode::Esc => {
                                pager.clear_selection();
                                pager.toggle_edit_mode();
                            }

                            // Ctrl+Q to quit from edit mode
                            KeyCode::Char('q') if ctrl => break,

                            // Clipboard operations
                            KeyCode::Char('c') if ctrl => pager.copy_selection(),
                            KeyCode::Char('x') if ctrl => pager.cut_selection(),
                            KeyCode::Char('v') if ctrl => pager.paste(),
                            KeyCode::Char('a') if ctrl => pager.select_all(),

                            // Arrow keys with Shift = extend selection
                            KeyCode::Up if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some((line, col)) = pager.layout_map.source_to_screen(current) {
                                        if line > 0 {
                                            if let Some(new_pos) = pager.layout_map.screen_to_source_nearest(line - 1, col) {
                                                pager.extend_selection_to(new_pos);
                                                pager.ensure_cursor_visible_with_viewport(viewport_height);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Down if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some((line, col)) = pager.layout_map.source_to_screen(current) {
                                        if line + 1 < pager.layout_map.line_count() {
                                            if let Some(new_pos) = pager.layout_map.screen_to_source_nearest(line + 1, col) {
                                                pager.extend_selection_to(new_pos);
                                                pager.ensure_cursor_visible_with_viewport(viewport_height);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Left if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some(prev) = pager.layout_map.prev_cursor_position(current) {
                                        pager.extend_selection_to(prev);
                                        pager.ensure_cursor_visible();
                                    }
                                }
                            }
                            KeyCode::Right if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some(next) = pager.layout_map.next_cursor_position(current) {
                                        pager.extend_selection_to(next);
                                        pager.ensure_cursor_visible();
                                    }
                                }
                            }

                            // Cursor movement without Shift = move and clear selection
                            KeyCode::Up => {
                                pager.clear_selection();
                                pager.cursor_up(viewport_height);
                            }
                            KeyCode::Down => {
                                pager.clear_selection();
                                pager.cursor_down(viewport_height);
                            }
                            KeyCode::Left => {
                                pager.clear_selection();
                                pager.cursor_left();
                            }
                            KeyCode::Right => {
                                pager.clear_selection();
                                pager.cursor_right();
                            }

                            // Editing keys
                            KeyCode::Backspace => pager.handle_backspace(),
                            KeyCode::Delete => pager.handle_delete(),
                            KeyCode::Enter => pager.handle_enter(),

                            // Ctrl+scroll shortcuts
                            KeyCode::Char('u') if ctrl => pager.scroll_up(viewport_height / 2),
                            KeyCode::Char('d') if ctrl => pager.scroll_down(viewport_height / 2, viewport_height),

                            // Page up/down
                            KeyCode::PageUp => pager.scroll_up(viewport_height),
                            KeyCode::PageDown => pager.scroll_down(viewport_height, viewport_height),

                            // Home/End for start/end of document
                            KeyCode::Home if ctrl => pager.scroll_to_top(),
                            KeyCode::End if ctrl => pager.scroll_to_bottom(viewport_height),

                            // Character input - insert the character
                            KeyCode::Char(ch) if !ctrl => pager.handle_char_input(ch),

                            _ => {}
                        }
                    } else {
                        // View mode (not editing) - vim-style navigation
                        match key.code {
                            // Quit
                            KeyCode::Char('q') | KeyCode::Esc => break,

                            // Enter edit mode with 'i' or 'e'
                            KeyCode::Char('i') | KeyCode::Char('e') => pager.toggle_edit_mode(),

                            // Arrow keys: cursor movement
                            KeyCode::Up => pager.cursor_up(viewport_height),
                            KeyCode::Down => pager.cursor_down(viewport_height),
                            KeyCode::Left => pager.cursor_left(),
                            KeyCode::Right => pager.cursor_right(),

                            // Vim-style scroll
                            KeyCode::Char('k') => pager.scroll_up(1),
                            KeyCode::Char('j') => pager.scroll_down(1, viewport_height),
                            KeyCode::Char('u') => pager.scroll_up(viewport_height / 2),
                            KeyCode::Char('d') => pager.scroll_down(viewport_height / 2, viewport_height),
                            KeyCode::PageUp | KeyCode::Char('b') => pager.scroll_up(viewport_height),
                            KeyCode::PageDown | KeyCode::Char(' ') => {
                                pager.scroll_down(viewport_height, viewport_height)
                            }

                            // Jump to top/bottom
                            KeyCode::Char('g') => pager.scroll_to_top(),
                            KeyCode::Char('G') => pager.scroll_to_bottom(viewport_height),
                            KeyCode::Home => pager.scroll_to_top(),
                            KeyCode::End => pager.scroll_to_bottom(viewport_height),

                            // Toggle cursor visibility
                            KeyCode::Char('c') => pager.toggle_cursor(),

                            _ => {}
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => pager.scroll_up(3),
                        MouseEventKind::ScrollDown => pager.scroll_down(3, viewport_height),
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            // Start drag selection
                            pager.start_mouse_drag(mouse.column, mouse.row);
                        }
                        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                            // Continue drag selection
                            pager.update_mouse_drag(mouse.column, mouse.row);
                        }
                        MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                            // End drag selection
                            pager.end_mouse_drag();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn draw(frame: &mut Frame, pager: &mut Pager, viewport_height: usize) {
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

    // Store content area for mouse click handling
    pager.set_content_area(content_area.x, content_area.y, content_area.width, content_area.height);

    let scroll_pos = pager.scroll_position();

    // Render paragraph with scroll (using reference, not clone)
    let paragraph = Paragraph::new(pager.content.clone())
        .scroll((scroll_pos as u16, 0));

    frame.render_widget(paragraph, content_area);

    // Render selection highlighting
    if pager.selection.is_active() {
        let selection_style = Style::default().add_modifier(Modifier::REVERSED);
        let selection_positions = pager.selection_screen_positions();

        for (line, col) in selection_positions {
            // Only highlight if within visible viewport
            if line >= scroll_pos && line < scroll_pos + viewport_height {
                let screen_y = content_area.y + (line - scroll_pos) as u16;
                let screen_x = content_area.x + col as u16;

                // Only highlight if within content area bounds
                if screen_x < content_area.x + content_area.width
                    && screen_y < content_area.y + content_area.height
                {
                    // Get the cell and apply selection style
                    let cell = frame.buffer_mut().cell_mut(Position::new(screen_x, screen_y));
                    if let Some(cell) = cell {
                        cell.set_style(selection_style);
                    }
                }
            }
        }
    }

    // Show cursor if visible and within viewport
    if pager.cursor_visible {
        if let Some((cursor_line, cursor_col)) = pager.cursor_screen_position() {
            // Check if cursor is within visible viewport
            if cursor_line >= scroll_pos && cursor_line < scroll_pos + viewport_height {
                let screen_y = content_area.y + (cursor_line - scroll_pos) as u16;
                let screen_x = content_area.x + cursor_col as u16;

                // Only show cursor if within content area bounds
                if screen_x < content_area.x + content_area.width
                    && screen_y < content_area.y + content_area.height
                {
                    frame.set_cursor_position(Position::new(screen_x, screen_y));
                }
            }
        }
    }

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

    // Status bar at bottom - shows mode and position info
    let mode_indicator = if pager.edit_mode {
        "EDIT"
    } else {
        "VIEW"
    };

    let dirty_indicator = if pager.is_dirty() { " [+]" } else { "" };

    // Show selection info or cursor position
    let position_info = if pager.selection.is_active() {
        let (start, end) = pager.selection.range();
        format!(" sel:{}-{} ({})", start, end, end - start)
    } else if let Some(offset) = pager.cursor_source {
        format!(" byte:{}", offset)
    } else {
        String::new()
    };

    let help_text = if pager.edit_mode {
        if pager.selection.is_active() {
            "Ctrl+C:copy  Ctrl+X:cut"
        } else {
            "Esc:view  Ctrl+Q:quit"
        }
    } else {
        "i:edit  q:quit  j/k:scroll"
    };

    let status = format!(
        " {} │ {}-{} of {}{}{} │ {} ",
        mode_indicator,
        scroll_pos + 1,
        (scroll_pos + viewport_height).min(pager.total_lines),
        pager.total_lines,
        dirty_indicator,
        position_info,
        help_text
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
