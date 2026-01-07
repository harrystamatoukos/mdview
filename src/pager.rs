use anyhow::Result;
use crossterm::{
    cursor::SetCursorStyle,
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
use crate::primitives::ByteOffset;
use crate::renderer;
use crate::selection::Selection;
use crate::theme::{Theme, LEFT_MARGIN};

/// Info about a detected list item
struct ListItemInfo {
    /// The marker to use for continuation (e.g., "- " or "2. ")
    marker: String,
    /// Length of the marker in the current line (for detecting empty items)
    marker_len: usize,
    /// Content after the marker
    content: String,
}

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
    cursor_source: Option<ByteOffset>,  // Source byte offset (None = no cursor)
    cursor_visible: bool,
    // Selection state
    selection: Selection,
    // Editor state for text manipulation
    editor: EditorState,
    terminal_width: u16,
    // Content area for mouse click handling (x, y, width, height)
    content_area: (u16, u16, u16, u16),
    // Mouse drag state for selection
    mouse_dragging: bool,
    // Double-click detection
    last_click_time: std::time::Instant,
    last_click_pos: (u16, u16),
    click_count: u8,
    // Edit mode (always on for fluid editing, can toggle to view mode with 'v')
    edit_mode: bool,
    // Visual cursor override - used after Enter to position cursor where content WILL appear
    // Cleared on first keystroke when actual content exists to map to
    cursor_visual_override: Option<(usize, usize)>,
    // Pending paragraph - when user clicks in empty space, stores (source_offset, visual_line, visual_col)
    // where source_offset is where to insert newlines, and visual position is where cursor appears
    pending_paragraph: Option<(ByteOffset, usize, usize)>,
    // Last input time for responsive polling
    last_input_time: std::time::Instant,
}

// Easing factor: higher = snappier, lower = smoother
const SCROLL_EASING: f64 = 0.3;  // Snappier for more responsive feel
// Threshold to snap to target (avoid endless micro-animations)
const SCROLL_SNAP_THRESHOLD: f64 = 0.5;
// Fast polling duration after input (ms) - keep fast for a while after typing
const FAST_POLL_DURATION_MS: u128 = 1000;
// Minimum poll time during active editing (ms) - 60fps for smoothness
const ACTIVE_POLL_MS: u64 = 16;
// Idle poll time (ms) - still responsive but saves CPU
const IDLE_POLL_MS: u64 = 50;

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
        let cursor_source = layout_map.first_offset().map(ByteOffset);

        // Initialize selection at cursor position
        let selection = Selection::new(cursor_source.unwrap_or(ByteOffset::ZERO));

        let now = std::time::Instant::now();
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
            content_area: (0, 0, 0, 0), // Will be set on first draw
            mouse_dragging: false,
            last_click_time: now,
            last_click_pos: (0, 0),
            click_count: 0,
            edit_mode: true, // Always start in edit mode for fluid experience
            cursor_visual_override: None,
            pending_paragraph: None,
            last_input_time: now,
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

    /// Mark that input was received (for responsive polling)
    fn mark_input(&mut self) {
        self.last_input_time = std::time::Instant::now();
    }

    /// Check if we should use fast polling (recently had input)
    fn needs_fast_poll(&self) -> bool {
        self.last_input_time.elapsed().as_millis() < FAST_POLL_DURATION_MS
    }

    /// Handle a character input for editing with smart punctuation
    /// Designed for maximum fluidity - no delays or pending states
    fn handle_char_input(&mut self, ch: char) {
        // Clear visual override - we now have real content to map to
        self.cursor_visual_override = None;

        // Handle pending paragraph - user clicked in empty space and is now typing
        if let Some((insert_offset, _visual_line, _visual_col)) = self.pending_paragraph.take() {
            // Insert newlines to create the paragraph at the clicked location
            self.editor.set_cursor(insert_offset);
            self.editor.insert_str("\n\n");
            // Cursor is now positioned after the newlines, ready for new content
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
        }

        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
        }

        // Sync editor cursor from current visual cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Smart punctuation - automatic typographic improvements (fast O(1) operation)
        let smart_ch = self.apply_smart_punctuation(ch);

        // Insert the character directly - no pending states for maximum responsiveness
        self.editor.insert_char(smart_ch);
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());

        // Re-render
        self.re_render();
    }

    /// Insert bold markers (**)
    fn insert_bold_markers(&mut self) {
        self.editor.insert_str("**");
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());
        self.re_render();
    }

    /// Insert italic marker (*)
    fn insert_italic_marker(&mut self) {
        self.editor.insert_char('*');
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());
        self.re_render();
    }

    /// Insert code marker (`)
    fn insert_code_marker(&mut self) {
        self.editor.insert_char('`');
        self.cursor_source = Some(self.editor.cursor());
        self.selection.move_to(self.editor.cursor());
        self.re_render();
    }

    /// Format selection or insert bold markers (Ctrl+B)
    fn format_selection_bold(&mut self) {
        if self.selection.is_active() {
            self.toggle_selection_formatting("**", "**");
        } else {
            self.insert_bold_markers();
        }
    }

    /// Format selection or insert italic marker (Ctrl+I)
    fn format_selection_italic(&mut self) {
        if self.selection.is_active() {
            self.toggle_selection_formatting("*", "*");
        } else {
            self.insert_italic_marker();
        }
    }

    /// Format selection or insert code marker (Ctrl+`)
    fn format_selection_code(&mut self) {
        if self.selection.is_active() {
            self.toggle_selection_formatting("`", "`");
        } else {
            self.insert_code_marker();
        }
    }

    /// Toggle formatting on selection - removes if present, adds if not
    fn toggle_selection_formatting(&mut self, open: &str, close: &str) {
        let (start, end) = self.selection.range();
        let content = self.editor.content();

        // Check if selection is already wrapped with the markers
        // We need to look OUTSIDE the selection for the markers
        let open_len = open.len();
        let close_len = close.len();

        // Check if there are markers just outside the selection
        let has_open_before = start.get() >= open_len && &content[start.get() - open_len..start.get()] == open;
        let has_close_after = end.get() + close_len <= content.len() && &content[end.get()..end.get() + close_len] == close;

        if has_open_before && has_close_after {
            // Already formatted - REMOVE the markers
            // Delete closing marker first (so start offset stays valid)
            self.editor.delete_range(end, end + close_len);
            // Delete opening marker
            self.editor.delete_range(start - open_len, start);

            // Move cursor to end of now-unformatted text
            let new_cursor = end - open_len;
            self.editor.set_cursor(new_cursor);
            self.cursor_source = Some(new_cursor);
            self.selection.move_to(new_cursor);
        } else {
            // Check if selection CONTAINS the markers (user selected the formatted text including markers)
            let selected_text = &content[start.get()..end.get()];
            if selected_text.starts_with(open) && selected_text.ends_with(close) && selected_text.len() >= open_len + close_len {
                // Selection includes markers - remove them by replacing with inner content
                let inner = &selected_text[open_len..selected_text.len() - close_len];
                let inner_owned = inner.to_string();

                // Delete the selected text and insert just the inner content
                self.editor.delete_range(start, end);
                self.editor.set_cursor(start);
                self.editor.insert_str(&inner_owned);

                let new_cursor = start + inner_owned.len();
                self.editor.set_cursor(new_cursor);
                self.cursor_source = Some(new_cursor);
                self.selection.move_to(new_cursor);
            } else {
                // Not formatted - ADD the markers
                // Insert closing marker first (so offsets stay valid)
                self.editor.set_cursor(end);
                self.editor.insert_str(close);

                // Insert opening marker
                self.editor.set_cursor(start);
                self.editor.insert_str(open);

                // Move cursor after the formatted content
                let new_cursor = end + open_len + close_len;
                self.editor.set_cursor(new_cursor);
                self.cursor_source = Some(new_cursor);
                self.selection.move_to(new_cursor);
            }
        }

        self.re_render();
    }

    /// Wrap current selection with opening and closing markers (no toggle)
    #[allow(dead_code)]
    fn wrap_selection(&mut self, open: &str, close: &str) {
        let (start, end) = self.selection.range();

        // Insert closing marker first (so offsets stay valid)
        self.editor.set_cursor(end);
        self.editor.insert_str(close);

        // Insert opening marker
        self.editor.set_cursor(start);
        self.editor.insert_str(open);

        // Move cursor after the formatted content
        let new_cursor = end + open.len() + close.len();
        self.editor.set_cursor(new_cursor);
        self.cursor_source = Some(new_cursor);

        // Clear selection and re-render
        self.selection.move_to(new_cursor);
        self.re_render();
    }

    /// Toggle heading level at current line (Ctrl+1/2/3)
    fn toggle_heading(&mut self, level: u8) {
        if let Some(cursor_pos) = self.cursor_source {
            let line_start = self.editor.line_start(cursor_pos);
            let line = self.editor.line_content(cursor_pos);

            // Determine current heading level
            let current_level = Self::detect_heading_level(line);

            // Build the new prefix
            let new_prefix = if level == 0 || current_level == Some(level) {
                // Remove heading (Ctrl+0 or toggle same level)
                String::new()
            } else {
                // Set to new level
                format!("{} ", "#".repeat(level as usize))
            };

            // Calculate how much of the old prefix to remove
            let old_prefix_len = if let Some(lvl) = current_level {
                lvl as usize + 1 // "## " is 3 chars for level 2
            } else {
                0
            };

            // Delete old prefix
            if old_prefix_len > 0 {
                self.editor.delete_range(line_start, line_start + old_prefix_len);
            }

            // Insert new prefix
            if !new_prefix.is_empty() {
                self.editor.insert_at(line_start, &new_prefix);
            }

            // Update cursor position
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
            self.re_render();
        }
    }

    /// Detect heading level from line content
    /// Returns Some(level) if line starts with # prefix, None otherwise
    fn detect_heading_level(line: &str) -> Option<u8> {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            return None;
        }

        // Count consecutive # characters
        let hash_count = trimmed.chars().take_while(|&c| c == '#').count();

        // Verify it's followed by space (valid markdown heading)
        if hash_count >= 1 && hash_count <= 6 {
            let rest = &trimmed[hash_count..];
            if rest.is_empty() || rest.starts_with(' ') {
                return Some(hash_count as u8);
            }
        }

        None
    }

    /// Undo the last edit operation (Ctrl+Z)
    fn handle_undo(&mut self) {
        if let Some(cursor_pos) = self.editor.undo() {
            self.cursor_source = Some(cursor_pos);
            self.selection.move_to(cursor_pos);
            self.re_render();
        }
    }

    /// Redo the last undone operation (Ctrl+Shift+Z or Ctrl+Y)
    fn handle_redo(&mut self) {
        if let Some(cursor_pos) = self.editor.redo() {
            self.cursor_source = Some(cursor_pos);
            self.selection.move_to(cursor_pos);
            self.re_render();
        }
    }

    /// Insert link at cursor or wrap selection (Ctrl+K)
    /// If selection: wraps selected text as link text, cursor moves to URL placeholder
    /// If no selection: inserts [text](url) template, cursor at "text"
    fn insert_link(&mut self) {
        if let Some(cursor_pos) = self.cursor_source {
            if self.selection.is_active() {
                // Wrap selection as link
                let (start, end) = self.selection.range();

                // Insert closing part first
                self.editor.set_cursor(end);
                self.editor.insert_str("](url)");

                // Insert opening bracket
                self.editor.set_cursor(start);
                self.editor.insert_str("[");

                // Position cursor at "url" (3 chars from end: "url)")
                // end + "[" (1) + "](" (2) = end + 3, plus we want to be at 'u', so end + 3
                let url_pos = end + 3;
                self.editor.set_cursor(url_pos);
                self.cursor_source = Some(url_pos);
                self.selection.move_to(url_pos);
            } else {
                // No selection: insert template
                self.editor.set_cursor(cursor_pos);
                self.editor.insert_str("[text](url)");

                // Position cursor at start of "text" (after '[')
                let text_pos = cursor_pos + 1;
                self.editor.set_cursor(text_pos);
                self.cursor_source = Some(text_pos);
                self.selection.move_to(text_pos);
            }
            self.re_render();
        }
    }

    /// Smart Enter handling for structured content
    /// - List items: continue list or exit if empty
    /// - Blockquotes: continue quote or exit if empty
    /// - Normal text: insert single newline (markdown wraps soft breaks)
    fn handle_smart_enter(&mut self) {
        // Clear any pending paragraph or visual override
        self.pending_paragraph = None;
        self.cursor_visual_override = None;

        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        let Some(cursor_pos) = self.cursor_source else { return };

        let line = self.editor.line_content(cursor_pos);
        let line_start = self.editor.line_start(cursor_pos);
        let cursor_in_line = cursor_pos - line_start;

        // Detect list context
        if let Some(list_info) = Self::detect_list_item(line) {
            let content_start = list_info.marker_len;
            let has_content = !list_info.content.trim().is_empty();

            if !has_content && cursor_in_line <= content_start {
                // Exit list: remove the marker and insert newline
                self.editor.delete_range(line_start, line_start + list_info.marker_len);
                self.editor.set_cursor(line_start);
                self.editor.insert_char('\n');
                self.cursor_source = Some(line_start + 1);
                self.selection.move_to(line_start + 1);
            } else {
                // Continue list: insert newline + marker
                self.editor.set_cursor(cursor_pos);
                self.editor.insert_char('\n');
                self.editor.insert_str(&list_info.marker);

                let new_cursor = cursor_pos + 1 + list_info.marker.len();
                self.cursor_source = Some(new_cursor);
                self.selection.move_to(new_cursor);
            }
            self.re_render();
            self.ensure_cursor_visible();
            return;
        }

        // Detect blockquote context
        if let Some(quote_marker) = Self::detect_blockquote(line) {
            let trimmed = line.trim_start();
            let quote_content = if trimmed.starts_with("> ") {
                &trimmed[2..]
            } else if trimmed.starts_with(">") {
                &trimmed[1..]
            } else {
                trimmed
            };

            if quote_content.trim().is_empty() {
                // Exit blockquote: remove the marker and insert newline
                let marker_end = if line.trim_start().starts_with("> ") {
                    line.len() - line.trim_start().len() + 2
                } else {
                    line.len() - line.trim_start().len() + 1
                };
                self.editor.delete_range(line_start, line_start + marker_end);
                self.editor.set_cursor(line_start);
                self.editor.insert_char('\n');
                self.cursor_source = Some(line_start + 1);
                self.selection.move_to(line_start + 1);
            } else {
                // Continue blockquote
                self.editor.set_cursor(cursor_pos);
                self.editor.insert_char('\n');
                self.editor.insert_str(&quote_marker);

                let new_cursor = cursor_pos + 1 + quote_marker.len();
                self.cursor_source = Some(new_cursor);
                self.selection.move_to(new_cursor);
            }
            self.re_render();
            self.ensure_cursor_visible();
            return;
        }

        // Normal text: just insert a single newline
        // For markdown, a single newline is a soft break (continues paragraph)
        // Double newline creates a new paragraph
        self.editor.set_cursor(cursor_pos);
        self.editor.insert_char('\n');
        let new_cursor = cursor_pos + 1;
        self.cursor_source = Some(new_cursor);
        self.selection.move_to(new_cursor);

        self.re_render();
        self.ensure_cursor_visible();
    }

    /// Detect if line is a list item
    /// Returns (marker including space, marker length, content after marker)
    fn detect_list_item(line: &str) -> Option<ListItemInfo> {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // Unordered list: -, *, or + followed by space
        if let Some(rest) = trimmed.strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let marker_char = trimmed.chars().next().unwrap();
            let marker = format!("{}{} ", " ".repeat(indent), marker_char);
            return Some(ListItemInfo {
                marker,
                marker_len: indent + 2, // marker char + space
                content: rest.to_string(),
            });
        }

        // Ordered list: number followed by . or ) and space
        let mut chars = trimmed.chars().peekable();
        let mut num = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_ascii_digit() {
                num.push(ch);
                chars.next();
            } else {
                break;
            }
        }

        if !num.is_empty() {
            if let Some(&delim) = chars.peek() {
                if delim == '.' || delim == ')' {
                    chars.next();
                    if chars.peek() == Some(&' ') {
                        chars.next();
                        let rest: String = chars.collect();
                        // For continuation, increment the number
                        let next_num: u64 = num.parse().unwrap_or(1) + 1;
                        let marker = format!("{}{}{} ", " ".repeat(indent), next_num, delim);
                        return Some(ListItemInfo {
                            marker,
                            marker_len: indent + num.len() + 2, // number + delimiter + space
                            content: rest,
                        });
                    }
                }
            }
        }

        None
    }

    /// Detect if line is a blockquote
    /// Returns the marker to use for continuation (e.g., "> ")
    fn detect_blockquote(line: &str) -> Option<String> {
        let trimmed = line.trim_start();
        if trimmed.starts_with("> ") {
            Some("> ".to_string())
        } else if trimmed.starts_with(">") {
            Some("> ".to_string())
        } else {
            None
        }
    }

    /// Handle Tab key - indent list item/blockquote, or insert spaces
    fn handle_tab(&mut self) {
        if let Some(cursor_pos) = self.cursor_source {
            let line = self.editor.line_content(cursor_pos);
            let line_start = self.editor.line_start(cursor_pos);

            // In a list item or blockquote - indent the whole line
            if Self::detect_list_item(line).is_some() || Self::detect_blockquote(line).is_some() {
                // Insert 2 spaces at line start (standard markdown indent)
                self.editor.insert_at(line_start, "  ");
                // Cursor position is adjusted by insert_at
                self.cursor_source = Some(self.editor.cursor());
                self.selection.move_to(self.editor.cursor());
                self.re_render();
            } else {
                // Not in a structured element - insert spaces at cursor
                // Use 4 spaces as a "tab" equivalent
                self.editor.set_cursor(cursor_pos);
                self.editor.insert_str("    ");
                self.cursor_source = Some(self.editor.cursor());
                self.selection.move_to(self.editor.cursor());
                self.re_render();
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Smart Punctuation (typographic improvements while typing)
    // ─────────────────────────────────────────────────────────────────────────

    /// Get the character before a byte offset in O(1) instead of O(n)
    /// Works by looking back at most 4 bytes (max UTF-8 char size)
    fn char_before(content: &str, byte_pos: usize) -> Option<char> {
        if byte_pos == 0 || byte_pos > content.len() {
            return None;
        }
        let start = byte_pos.saturating_sub(4);
        content[start..byte_pos].chars().next_back()
    }

    /// Apply smart punctuation transformations for beautiful typography
    /// - Straight quotes → curly quotes (" → " or ")
    /// - Straight apostrophes → curly apostrophes (' → ')
    /// Fast O(1) lookups only - no content modification for speed
    fn apply_smart_punctuation(&self, ch: char) -> char {
        let content = self.editor.content();
        let cursor = self.cursor_source.unwrap_or(ByteOffset::ZERO);

        match ch {
            // Smart double quotes
            '"' => {
                // Opening quote if at start, after space/newline/punctuation, or after opening bracket
                // O(1) lookup via char_before
                let before = Self::char_before(content, cursor.get());

                match before {
                    None | Some(' ') | Some('\n') | Some('\t') | Some('(') | Some('[') | Some('{') => '\u{201C}', // " opening
                    _ => '\u{201D}', // " closing
                }
            }

            // Smart single quotes / apostrophes
            '\'' => {
                let before = Self::char_before(content, cursor.get());

                match before {
                    None | Some(' ') | Some('\n') | Some('\t') | Some('(') | Some('[') | Some('{') => '\u{2018}', // ' opening
                    _ => '\u{2019}', // ' closing/apostrophe
                }
            }

            // No transformation for other characters - keep it fast
            _ => ch,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Word and line navigation (for fluid editing like Google Docs)
    // ─────────────────────────────────────────────────────────────────────────

    /// Move cursor to previous word boundary (Ctrl+Left)
    fn cursor_word_left(&mut self) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            let new_pos = self.editor.word_start(current);
            self.cursor_source = Some(new_pos);
            self.editor.set_cursor(new_pos);
            self.ensure_cursor_visible();
        }
    }

    /// Move cursor to next word boundary (Ctrl+Right)
    fn cursor_word_right(&mut self) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            let new_pos = self.editor.word_end(current);
            self.cursor_source = Some(new_pos);
            self.editor.set_cursor(new_pos);
            self.ensure_cursor_visible();
        }
    }

    /// Move cursor to start of line (Home key)
    fn cursor_line_start(&mut self) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            let new_pos = self.editor.line_start(current);
            self.cursor_source = Some(new_pos);
            self.editor.set_cursor(new_pos);
            self.ensure_cursor_visible();
        }
    }

    /// Move cursor to end of line (End key)
    fn cursor_line_end(&mut self) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            let new_pos = self.editor.line_end(current);
            self.cursor_source = Some(new_pos);
            self.editor.set_cursor(new_pos);
            self.ensure_cursor_visible();
        }
    }

    /// Delete previous word (Ctrl+Backspace)
    fn handle_delete_word_before(&mut self) {
        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        if self.editor.delete_word_before() {
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
            self.re_render();
        }
    }

    /// Delete next word (Ctrl+Delete)
    fn handle_delete_word_after(&mut self) {
        // Delete selection first if any
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        if self.editor.delete_word_after() {
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
            self.re_render();
        }
    }

    /// Select word at current cursor position (for double-click)
    fn select_word_at_cursor(&mut self) {
        if let Some(current) = self.cursor_source {
            let (start, end) = self.editor.word_at(current);
            if start < end {
                self.selection = Selection::from_range(start, end);
                self.cursor_source = Some(end);
                self.editor.set_cursor(end);
            }
        }
    }

    /// Select entire line/paragraph at cursor (for triple-click)
    fn select_line_at_cursor(&mut self) {
        if let Some(current) = self.cursor_source {
            let start = self.editor.line_start(current);
            let end = self.editor.line_end(current);
            // Include the newline if there is one
            let end = if end.get() < self.editor.len() { end + 1 } else { end };
            self.selection = Selection::from_range(start, end);
            self.cursor_source = Some(end);
            self.editor.set_cursor(end);
        }
    }

    /// Handle mouse click with double/triple click detection
    /// Supports clicking in empty space to create pending paragraphs
    fn handle_mouse_click(&mut self, screen_x: u16, screen_y: u16) {
        // Clear overrides - user is clicking to position
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        let now = std::time::Instant::now();
        let (area_x, area_y, area_width, area_height) = self.content_area;

        // Check if click is within content area
        if screen_x < area_x || screen_x >= area_x + area_width
            || screen_y < area_y || screen_y >= area_y + area_height
        {
            return;
        }

        // Double/triple click detection (within 400ms and same position)
        let is_same_pos = (screen_x as i32 - self.last_click_pos.0 as i32).abs() <= 2
            && (screen_y as i32 - self.last_click_pos.1 as i32).abs() <= 2;
        let is_quick_click = now.duration_since(self.last_click_time).as_millis() < 400;

        if is_same_pos && is_quick_click {
            self.click_count = (self.click_count % 3) + 1;
        } else {
            self.click_count = 1;
        }

        self.last_click_time = now;
        self.last_click_pos = (screen_x, screen_y);

        // Convert screen position to content position
        let content_col = (screen_x - area_x) as usize;
        let scroll_pos = self.scroll_position();
        let content_line = scroll_pos + (screen_y - area_y) as usize;

        // Check if there's actual content on this line
        let line_has_content = self.layout_map.line_has_content(content_line);

        if line_has_content {
            // Normal click - find nearest source position on this line
            if let Some(offset) = self.layout_map.screen_to_source_nearest(content_line, content_col) {
                let offset = ByteOffset(offset);
                self.cursor_source = Some(offset);
                self.editor.set_cursor(offset);

                match self.click_count {
                    1 => {
                        // Single click - position cursor, clear selection
                        self.selection = Selection::new(offset);
                    }
                    2 => {
                        // Double click - select word
                        self.select_word_at_cursor();
                    }
                    3 => {
                        // Triple click - select line/paragraph
                        self.select_line_at_cursor();
                    }
                    _ => {}
                }
            }
        } else {
            // Click in empty space - create pending paragraph
            // Find the nearest content above to determine insertion point
            if let Some(insert_offset) = self.layout_map.find_insertion_point_before_line(content_line) {
                let insert_offset = ByteOffset(insert_offset);
                // Use content margin for column position in empty space
                let visual_col = self.layout_map.content_margin();
                self.pending_paragraph = Some((insert_offset, content_line, visual_col));
                self.selection = Selection::new(insert_offset);
            }
        }
    }

    /// Handle Shift+Tab key - outdent list item or remove leading indentation
    fn handle_shift_tab(&mut self) {
        if let Some(cursor_pos) = self.cursor_source {
            let line = self.editor.line_content(cursor_pos);
            let line_start = self.editor.line_start(cursor_pos);

            // Calculate leading whitespace
            let leading_spaces = line.len() - line.trim_start().len();

            if leading_spaces >= 2 {
                // Remove up to 2 spaces from line start (for lists) or 4 (for regular text)
                let spaces_to_remove = if Self::detect_list_item(line).is_some() || Self::detect_blockquote(line).is_some() {
                    2.min(leading_spaces)
                } else {
                    4.min(leading_spaces)
                };
                self.editor.delete_range(line_start, line_start + spaces_to_remove);
                // Update cursor position
                self.cursor_source = Some(self.editor.cursor());
                self.selection.move_to(self.editor.cursor());
                self.re_render();
            }
        }
    }

    /// Handle backspace (delete before cursor)
    /// Simple and fast for fluid editing
    fn handle_backspace(&mut self) {
        // Clear visual override - user is actively editing
        self.cursor_visual_override = None;
        self.pending_paragraph = None;

        // If there's a selection, delete it instead
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Simple delete - no complex formatting boundary detection for speed
        if self.editor.delete_before() {
            self.cursor_source = Some(self.editor.cursor());
            self.selection.move_to(self.editor.cursor());
            self.re_render();
        }
    }

    /// Handle delete key (delete at cursor)
    /// Simple and fast for fluid editing
    fn handle_delete(&mut self) {
        // Clear visual override - user is actively editing
        self.cursor_visual_override = None;
        self.pending_paragraph = None;

        // If there's a selection, delete it instead
        if self.selection.is_active() {
            self.delete_selection();
            return;
        }

        // Sync editor cursor
        if let Some(offset) = self.cursor_source {
            self.editor.set_cursor(offset);
        }

        // Simple delete - no complex formatting boundary detection for speed
        if self.editor.delete_at() {
            self.cursor_source = Some(self.editor.cursor());
            self.re_render();
        }
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
        if end.get() <= content.len() {
            Some(content[start.get()..end.get()].to_string())
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
    fn extend_selection_to(&mut self, new_cursor: ByteOffset) {
        self.selection.extend_to(new_cursor);
        self.cursor_source = Some(new_cursor);
    }

    /// Move cursor and clear selection (for Arrow without Shift)
    fn move_cursor_to(&mut self, new_cursor: ByteOffset) {
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
            let offset = ByteOffset(offset);
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
            let offset = ByteOffset(offset);
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
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            if let Some(next) = self.layout_map.next_cursor_position(current.get()) {
                let next = ByteOffset(next);
                self.cursor_source = Some(next);
                self.editor.set_cursor(next);
                self.ensure_cursor_visible();
            }
        }
    }

    /// Move cursor to previous valid position (left arrow)
    fn cursor_left(&mut self) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            if let Some(prev) = self.layout_map.prev_cursor_position(current.get()) {
                let prev = ByteOffset(prev);
                self.cursor_source = Some(prev);
                self.editor.set_cursor(prev);
                self.ensure_cursor_visible();
            }
        }
    }

    /// Move cursor up one line - fluid movement like Google Docs
    fn cursor_up(&mut self, viewport_height: usize) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            if let Some((line, col)) = self.layout_map.source_to_screen(current.get()) {
                if line > 0 {
                    // Try to maintain column position on line above
                    if let Some(offset) = self.layout_map.screen_to_source_nearest(line - 1, col) {
                        let offset = ByteOffset(offset);
                        self.cursor_source = Some(offset);
                        self.editor.set_cursor(offset);
                        self.ensure_cursor_visible_with_viewport(viewport_height);
                    }
                }
            }
        }
    }

    /// Move cursor down one line - fluid movement like Google Docs
    fn cursor_down(&mut self, viewport_height: usize) {
        self.cursor_visual_override = None;
        self.pending_paragraph = None;
        if let Some(current) = self.cursor_source {
            if let Some((line, col)) = self.layout_map.source_to_screen(current.get()) {
                if line + 1 < self.layout_map.line_count() {
                    // Try to maintain column position on line below
                    if let Some(offset) = self.layout_map.screen_to_source_nearest(line + 1, col) {
                        let offset = ByteOffset(offset);
                        self.cursor_source = Some(offset);
                        self.editor.set_cursor(offset);
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
            if let Some((line, _)) = self.layout_map.source_to_screen(current.get()) {
                let scroll_pos = self.scroll_position();

                // Add margin to keep cursor away from edges (better UX)
                let scroll_margin = 3.min(viewport_height / 4);

                // Scroll up if cursor is near top of viewport
                if line < scroll_pos + scroll_margin {
                    self.scroll_target = line.saturating_sub(scroll_margin) as f64;
                }
                // Scroll down if cursor is near bottom of viewport
                else if line >= scroll_pos + viewport_height - scroll_margin {
                    self.scroll_target = (line.saturating_sub(viewport_height - scroll_margin - 1)) as f64;
                }
            }
        }
    }

    /// Get cursor screen position (line, col) relative to content
    /// Uses visual override if set (e.g., after Enter, before first keystroke)
    fn cursor_screen_position(&self) -> Option<(usize, usize)> {
        // Visual override takes precedence (used after Enter to show where content WILL appear)
        if let Some(override_pos) = self.cursor_visual_override {
            return Some(override_pos);
        }
        // Pending paragraph position (user clicked in empty space)
        if let Some((_, line, col)) = self.pending_paragraph {
            return Some((line, col));
        }
        self.cursor_source.and_then(|offset| self.layout_map.source_to_screen(offset.get()))
    }

    /// Get screen positions for active selection (for highlighting)
    fn selection_screen_positions(&self) -> Vec<(usize, usize)> {
        if !self.selection.is_active() {
            return Vec::new();
        }
        let (start, end) = self.selection.range();
        self.layout_map.source_range_to_screen(start.get(), end.get())
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
            let offset = ByteOffset(offset);
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
    // Setup terminal with mouse support and typewriter-style block cursor
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetCursorStyle::BlinkingBlock  // Typewriter-style cursor
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Get terminal size and create editor state
    let size = terminal.size()?;
    let editor = EditorState::new(content.to_string());
    let mut pager = Pager::new(editor, size.width, theme);

    let result = run_event_loop(&mut terminal, &mut pager);

    // Cleanup - restore default cursor
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        SetCursorStyle::DefaultUserShape  // Restore user's default cursor
    )?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pager: &mut Pager,
) -> Result<()> {
    loop {
        let viewport_height = terminal.size()?.height.saturating_sub(2) as usize;

        // Update scroll animation
        let animating = pager.update_animation();

        terminal.draw(|frame| draw(frame, pager, viewport_height))?;

        // Responsive polling: fast when recently active or animating, slower when idle
        let poll_timeout = if animating || pager.needs_fast_poll() {
            std::time::Duration::from_millis(ACTIVE_POLL_MS) // ~60fps for responsiveness
        } else {
            std::time::Duration::from_millis(IDLE_POLL_MS) // Idle but still responsive
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    pager.mark_input(); // Reset cursor blink and enable fast polling
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

                    // Handle based on edit mode
                    if pager.is_edit_mode() {
                        match key.code {
                            // Escape clears selection (like Google Docs)
                            // Double-Escape or Escape when no selection enters view mode
                            KeyCode::Esc => {
                                if pager.selection.is_active() {
                                    pager.clear_selection();
                                } else {
                                    pager.toggle_edit_mode();
                                }
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
                                    if let Some((line, col)) = pager.layout_map.source_to_screen(current.get()) {
                                        if line > 0 {
                                            if let Some(new_pos) = pager.layout_map.screen_to_source_nearest(line - 1, col) {
                                                pager.extend_selection_to(ByteOffset(new_pos));
                                                pager.ensure_cursor_visible_with_viewport(viewport_height);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Down if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some((line, col)) = pager.layout_map.source_to_screen(current.get()) {
                                        if line + 1 < pager.layout_map.line_count() {
                                            if let Some(new_pos) = pager.layout_map.screen_to_source_nearest(line + 1, col) {
                                                pager.extend_selection_to(ByteOffset(new_pos));
                                                pager.ensure_cursor_visible_with_viewport(viewport_height);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Left if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some(prev) = pager.layout_map.prev_cursor_position(current.get()) {
                                        pager.extend_selection_to(ByteOffset(prev));
                                        pager.ensure_cursor_visible();
                                    }
                                }
                            }
                            KeyCode::Right if shift => {
                                if let Some(current) = pager.cursor_source {
                                    if let Some(next) = pager.layout_map.next_cursor_position(current.get()) {
                                        pager.extend_selection_to(ByteOffset(next));
                                        pager.ensure_cursor_visible();
                                    }
                                }
                            }

                            // Word-level movement with Ctrl (like Google Docs)
                            KeyCode::Left if ctrl && shift => {
                                // Ctrl+Shift+Left = extend selection to previous word
                                if let Some(current) = pager.cursor_source {
                                    let new_pos = pager.editor.word_start(current);
                                    pager.extend_selection_to(new_pos);
                                    pager.ensure_cursor_visible();
                                }
                            }
                            KeyCode::Right if ctrl && shift => {
                                // Ctrl+Shift+Right = extend selection to next word
                                if let Some(current) = pager.cursor_source {
                                    let new_pos = pager.editor.word_end(current);
                                    pager.extend_selection_to(new_pos);
                                    pager.ensure_cursor_visible();
                                }
                            }
                            KeyCode::Left if ctrl => {
                                // Ctrl+Left = jump to previous word
                                pager.clear_selection();
                                pager.cursor_word_left();
                            }
                            KeyCode::Right if ctrl => {
                                // Ctrl+Right = jump to next word
                                pager.clear_selection();
                                pager.cursor_word_right();
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

                            // Home/End for line navigation (like Google Docs)
                            KeyCode::Home if shift => {
                                // Shift+Home = extend selection to line start
                                if let Some(current) = pager.cursor_source {
                                    let line_start = pager.editor.line_start(current);
                                    pager.extend_selection_to(line_start);
                                    pager.ensure_cursor_visible();
                                }
                            }
                            KeyCode::End if shift => {
                                // Shift+End = extend selection to line end
                                if let Some(current) = pager.cursor_source {
                                    let line_end = pager.editor.line_end(current);
                                    pager.extend_selection_to(line_end);
                                    pager.ensure_cursor_visible();
                                }
                            }
                            KeyCode::Home if ctrl => {
                                // Ctrl+Home = jump to document start
                                pager.clear_selection();
                                pager.cursor_source = Some(ByteOffset::ZERO);
                                pager.editor.set_cursor(ByteOffset::ZERO);
                                pager.selection.move_to(ByteOffset::ZERO);
                                pager.scroll_to_top();
                            }
                            KeyCode::End if ctrl => {
                                // Ctrl+End = jump to document end
                                pager.clear_selection();
                                let end = ByteOffset(pager.editor.len());
                                pager.cursor_source = Some(end);
                                pager.editor.set_cursor(end);
                                pager.selection.move_to(end);
                                pager.scroll_to_bottom(viewport_height);
                            }
                            KeyCode::Home => {
                                // Home = jump to line start
                                pager.clear_selection();
                                pager.cursor_line_start();
                            }
                            KeyCode::End => {
                                // End = jump to line end
                                pager.clear_selection();
                                pager.cursor_line_end();
                            }

                            // Editing keys
                            KeyCode::Backspace if ctrl => pager.handle_delete_word_before(),
                            KeyCode::Backspace => pager.handle_backspace(),
                            KeyCode::Delete if ctrl => pager.handle_delete_word_after(),
                            KeyCode::Delete => pager.handle_delete(),
                            KeyCode::Enter => pager.handle_smart_enter(),

                            // Ctrl+scroll shortcuts
                            KeyCode::Char('u') if ctrl => pager.scroll_up(viewport_height / 2),
                            KeyCode::Char('d') if ctrl => pager.scroll_down(viewport_height / 2, viewport_height),

                            // Page up/down
                            KeyCode::PageUp => pager.scroll_up(viewport_height),
                            KeyCode::PageDown => pager.scroll_down(viewport_height, viewport_height),

                            // Formatting shortcuts
                            KeyCode::Char('b') if ctrl => pager.format_selection_bold(),
                            KeyCode::Char('i') if ctrl => pager.format_selection_italic(),
                            KeyCode::Char('`') if ctrl => pager.format_selection_code(),

                            // Heading shortcuts (Ctrl+1/2/3 to set, Ctrl+0 to remove)
                            KeyCode::Char('1') if ctrl => pager.toggle_heading(1),
                            KeyCode::Char('2') if ctrl => pager.toggle_heading(2),
                            KeyCode::Char('3') if ctrl => pager.toggle_heading(3),
                            KeyCode::Char('0') if ctrl => pager.toggle_heading(0),

                            // Link insertion (Ctrl+K)
                            KeyCode::Char('k') if ctrl => pager.insert_link(),

                            // Undo/Redo
                            KeyCode::Char('z') if ctrl && shift => pager.handle_redo(),
                            KeyCode::Char('z') if ctrl => pager.handle_undo(),
                            KeyCode::Char('y') if ctrl => pager.handle_redo(),

                            // Tab/Shift+Tab for list nesting
                            KeyCode::Tab if shift => pager.handle_shift_tab(),
                            KeyCode::Tab => pager.handle_tab(),
                            KeyCode::BackTab => pager.handle_shift_tab(), // Some terminals send BackTab

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
                    pager.mark_input(); // Reset cursor blink on mouse activity
                    match mouse.kind {
                        MouseEventKind::ScrollUp => pager.scroll_up(3),
                        MouseEventKind::ScrollDown => pager.scroll_down(3, viewport_height),
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            // Handle click with double/triple click detection
                            pager.handle_mouse_click(mouse.column, mouse.row);
                            // Start potential drag if single click
                            if pager.click_count == 1 {
                                pager.start_mouse_drag(mouse.column, mouse.row);
                            }
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

    // Render selection highlighting - clean, subtle highlight
    if pager.selection.is_active() {
        // Use a subtle highlight that doesn't obscure text - inverted works well
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
                    // Apply selection style to cell
                    if let Some(cell) = frame.buffer_mut().cell_mut(Position::new(screen_x, screen_y)) {
                        cell.set_style(selection_style);
                    }
                }
            }
        }
    }

    // Show cursor if visible and within viewport (terminal handles blink animation)
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

    // Status bar at bottom - clean, minimal, helpful
    let dirty_indicator = if pager.is_dirty() { "• " } else { "" };

    // Word count for writers
    let word_count = pager.editor.word_count();

    // Selection info if active
    let selection_info = if pager.selection.is_active() {
        let (start, end) = pager.selection.range();
        format!(" │ {} selected", end - start)
    } else {
        String::new()
    };

    // Context-aware help - shorter, cleaner
    let help_text = if pager.edit_mode {
        if pager.selection.is_active() {
            "⌘C copy  ⌘X cut"
        } else {
            "Esc view  ⌘Q quit"
        }
    } else {
        "i edit  q quit"
    };

    let status = format!(
        " {}{} words{} │ {} ",
        dirty_indicator,
        word_count,
        selection_info,
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
