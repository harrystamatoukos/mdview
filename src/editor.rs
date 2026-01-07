//! Editor state for WYSIWYG markdown editing
//!
//! This module manages the mutable content state, handling insertions,
//! deletions, and cursor position in terms of byte offsets.

/// Mutable editor state holding the markdown content
#[derive(Debug, Clone)]
pub struct EditorState {
    /// The markdown content being edited
    content: String,
    /// Cursor position as byte offset in content
    cursor: usize,
    /// Whether content has been modified since last save
    dirty: bool,
}

impl EditorState {
    /// Create a new editor state from content
    pub fn new(content: String) -> Self {
        Self {
            content,
            cursor: 0,
            dirty: false,
        }
    }

    /// Get the content
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Get current cursor position (byte offset)
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Set cursor position (clamped to valid range)
    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.min(self.content.len());
    }

    /// Check if content has been modified
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark content as saved (clear dirty flag)
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Get content length in bytes
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Check if content is empty
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Editing operations
    // ─────────────────────────────────────────────────────────────────────────

    /// Insert a character at the cursor position
    /// Returns the new cursor position
    pub fn insert_char(&mut self, ch: char) -> usize {
        // Ensure cursor is at a valid UTF-8 boundary
        let cursor = self.ensure_char_boundary(self.cursor);

        self.content.insert(cursor, ch);
        self.cursor = cursor + ch.len_utf8();
        self.dirty = true;

        self.cursor
    }

    /// Insert a string at the cursor position
    /// Returns the new cursor position
    pub fn insert_str(&mut self, s: &str) -> usize {
        let cursor = self.ensure_char_boundary(self.cursor);

        self.content.insert_str(cursor, s);
        self.cursor = cursor + s.len();
        self.dirty = true;

        self.cursor
    }

    /// Insert a newline at cursor position
    /// Returns the new cursor position
    pub fn insert_newline(&mut self) -> usize {
        self.insert_char('\n')
    }

    /// Delete the character before cursor (backspace)
    /// Returns true if a character was deleted
    pub fn delete_before(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }

        // Find the start of the previous character by going back 1 byte first
        // then finding the char boundary (handles multi-byte UTF-8)
        let mut prev_char_start = self.cursor - 1;
        while prev_char_start > 0 && !self.content.is_char_boundary(prev_char_start) {
            prev_char_start -= 1;
        }

        // Remove the character
        self.content.drain(prev_char_start..self.cursor);
        self.cursor = prev_char_start;
        self.dirty = true;

        true
    }

    /// Delete the character at cursor position (delete key)
    /// Returns true if a character was deleted
    pub fn delete_at(&mut self) -> bool {
        if self.cursor >= self.content.len() {
            return false;
        }

        let cursor = self.ensure_char_boundary(self.cursor);
        let next_char_end = self.next_char_boundary(cursor);

        self.content.drain(cursor..next_char_end);
        self.dirty = true;

        true
    }

    /// Delete a range of content
    pub fn delete_range(&mut self, start: usize, end: usize) {
        let start = self.ensure_char_boundary(start);
        let end = self.ensure_char_boundary(end.min(self.content.len()));

        if start < end {
            self.content.drain(start..end);
            self.dirty = true;

            // Adjust cursor if it was in the deleted range
            if self.cursor > start {
                if self.cursor >= end {
                    self.cursor -= end - start;
                } else {
                    self.cursor = start;
                }
            }
        }
    }

    /// Replace all content (e.g., after reload)
    pub fn replace_content(&mut self, new_content: String) {
        self.content = new_content;
        self.cursor = self.cursor.min(self.content.len());
        // Don't mark dirty - this is a reload, not an edit
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor movement helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Move cursor to the next character
    pub fn move_cursor_forward(&mut self) {
        if self.cursor < self.content.len() {
            self.cursor = self.next_char_boundary(self.cursor);
        }
    }

    /// Move cursor to the previous character
    pub fn move_cursor_back(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_char_boundary(self.cursor);
        }
    }

    /// Move cursor to start of content
    pub fn move_cursor_to_start(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end of content
    pub fn move_cursor_to_end(&mut self) {
        self.cursor = self.content.len();
    }

    // ─────────────────────────────────────────────────────────────────────────
    // UTF-8 boundary helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Ensure position is at a valid UTF-8 character boundary
    fn ensure_char_boundary(&self, pos: usize) -> usize {
        let pos = pos.min(self.content.len());

        // If already at boundary, return as-is
        if self.content.is_char_boundary(pos) {
            return pos;
        }

        // Search backward for the nearest boundary
        self.prev_char_boundary(pos)
    }

    /// Find the previous character boundary
    fn prev_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.min(self.content.len());
        while p > 0 && !self.content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// Find the next character boundary
    fn next_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos;
        while p < self.content.len() && !self.content.is_char_boundary(p) {
            p += 1;
        }
        // Move past the current character
        if p < self.content.len() {
            p += 1;
            while p < self.content.len() && !self.content.is_char_boundary(p) {
                p += 1;
            }
        }
        p
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_editor() {
        let editor = EditorState::new("Hello".to_string());
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), 0);
        assert!(!editor.is_dirty());
    }

    #[test]
    fn test_insert_char() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(5);
        editor.insert_char('!');
        assert_eq!(editor.content(), "Hello!");
        assert_eq!(editor.cursor(), 6);
        assert!(editor.is_dirty());
    }

    #[test]
    fn test_insert_char_middle() {
        let mut editor = EditorState::new("Hllo".to_string());
        editor.set_cursor(1);
        editor.insert_char('e');
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), 2);
    }

    #[test]
    fn test_delete_before() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(5);
        assert!(editor.delete_before());
        assert_eq!(editor.content(), "Hell");
        assert_eq!(editor.cursor(), 4);
    }

    #[test]
    fn test_delete_before_at_start() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(0);
        assert!(!editor.delete_before());
        assert_eq!(editor.content(), "Hello");
    }

    #[test]
    fn test_delete_at() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(0);
        assert!(editor.delete_at());
        assert_eq!(editor.content(), "ello");
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn test_delete_at_end() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(5);
        assert!(!editor.delete_at());
        assert_eq!(editor.content(), "Hello");
    }

    #[test]
    fn test_insert_newline() {
        let mut editor = EditorState::new("HelloWorld".to_string());
        editor.set_cursor(5);
        editor.insert_newline();
        assert_eq!(editor.content(), "Hello\nWorld");
        assert_eq!(editor.cursor(), 6);
    }

    #[test]
    fn test_utf8_handling() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(5);
        editor.insert_char('世');  // Multi-byte character
        assert_eq!(editor.content(), "Hello世");
        assert_eq!(editor.cursor(), 8);  // 5 + 3 bytes for 世

        editor.delete_before();
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), 5);
    }

    #[test]
    fn test_delete_range() {
        let mut editor = EditorState::new("Hello World".to_string());
        editor.set_cursor(8);  // Cursor at 'r'
        editor.delete_range(5, 11);  // Delete " World"
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), 5);  // Cursor adjusted
    }
}
