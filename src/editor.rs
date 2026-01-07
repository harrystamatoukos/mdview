//! Editor state for WYSIWYG markdown editing
//!
//! This module manages the mutable content state, handling insertions,
//! deletions, and cursor position in terms of byte offsets.

use std::collections::VecDeque;
use std::time::Instant;

use crate::primitives::ByteOffset;

// ═══════════════════════════════════════════════════════════════════════════
// UNDO/REDO HISTORY
// ═══════════════════════════════════════════════════════════════════════════

/// An edit operation that can be undone/redone
#[derive(Debug, Clone)]
pub enum EditOperation {
    /// Text was inserted at position
    Insert { pos: ByteOffset, text: String },
    /// Text was deleted from position
    Delete { pos: ByteOffset, text: String },
}

impl EditOperation {
    /// Create the inverse operation (for undo)
    fn inverse(&self) -> EditOperation {
        match self {
            EditOperation::Insert { pos, text } => EditOperation::Delete {
                pos: *pos,
                text: text.clone(),
            },
            EditOperation::Delete { pos, text } => EditOperation::Insert {
                pos: *pos,
                text: text.clone(),
            },
        }
    }
}

/// Undo/redo history stack
#[derive(Debug, Clone)]
pub struct History {
    /// Operations that can be undone (VecDeque for O(1) pop_front)
    undos: VecDeque<EditOperation>,
    /// Operations that can be redone
    redos: Vec<EditOperation>,
    /// Maximum number of operations to keep
    max_size: usize,
    /// Last operation time (for coalescing)
    last_op_time: Option<Instant>,
    /// Coalesce threshold in milliseconds
    coalesce_ms: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undos: VecDeque::new(),
            redos: Vec::new(),
            max_size: 1000,
            last_op_time: None,
            coalesce_ms: 500, // Coalesce operations within 500ms
        }
    }
}

impl History {
    /// Record an operation (clears redo stack)
    fn record(&mut self, op: EditOperation) {
        let now = Instant::now();

        // Try to coalesce with previous operation
        if let Some(last_time) = self.last_op_time {
            if now.duration_since(last_time).as_millis() < self.coalesce_ms as u128 {
                if let Some(last_op) = self.undos.back_mut() {
                    if Self::can_coalesce(last_op, &op) {
                        Self::coalesce(last_op, op);
                        self.last_op_time = Some(now);
                        return;
                    }
                }
            }
        }

        // Add new operation
        self.undos.push_back(op);
        self.redos.clear();

        // Trim history if too large (O(1) with VecDeque)
        if self.undos.len() > self.max_size {
            self.undos.pop_front();
        }

        self.last_op_time = Some(now);
    }

    /// Check if two operations can be coalesced
    fn can_coalesce(prev: &EditOperation, next: &EditOperation) -> bool {
        match (prev, next) {
            // Coalesce consecutive insertions at adjacent positions
            (
                EditOperation::Insert { pos: p1, text: t1 },
                EditOperation::Insert { pos: p2, .. },
            ) => *p2 == *p1 + t1.len(),
            // Coalesce consecutive deletions at same position (backspace)
            (
                EditOperation::Delete { pos: p1, .. },
                EditOperation::Delete { pos: p2, .. },
            ) => *p1 == *p2 || *p1 == *p2 + 1,
            _ => false,
        }
    }

    /// Coalesce next operation into prev
    fn coalesce(prev: &mut EditOperation, next: EditOperation) {
        match (prev, next) {
            (
                EditOperation::Insert { text: t1, .. },
                EditOperation::Insert { text: t2, .. },
            ) => {
                t1.push_str(&t2);
            }
            (
                EditOperation::Delete { pos: p1, text: t1 },
                EditOperation::Delete { pos: p2, text: t2 },
            ) => {
                if *p1 == p2 + t2.len() {
                    // Backspace: prepend deleted text
                    *p1 = p2;
                    let mut new_text = t2;
                    new_text.push_str(t1);
                    *t1 = new_text;
                } else {
                    // Delete key: append deleted text
                    t1.push_str(&t2);
                }
            }
            _ => {}
        }
    }

    /// Pop an operation for undo, push to redo
    fn pop_undo(&mut self) -> Option<EditOperation> {
        self.undos.pop_back().map(|op| {
            let inverse = op.inverse();
            self.redos.push(inverse.clone());
            inverse
        })
    }

    /// Pop an operation for redo, push to undo
    fn pop_redo(&mut self) -> Option<EditOperation> {
        self.redos.pop().map(|op| {
            let inverse = op.inverse();
            self.undos.push_back(inverse.clone());
            inverse
        })
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undos.is_empty()
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redos.is_empty()
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.undos.clear();
        self.redos.clear();
        self.last_op_time = None;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// EDITOR STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Mutable editor state holding the markdown content
#[derive(Debug, Clone)]
pub struct EditorState {
    /// The markdown content being edited
    content: String,
    /// Cursor position as byte offset in content
    cursor: ByteOffset,
    /// Whether content has been modified since last save
    dirty: bool,
    /// Undo/redo history
    history: History,
}

impl EditorState {
    /// Create a new editor state from content
    pub fn new(content: String) -> Self {
        Self {
            content,
            cursor: ByteOffset::ZERO,
            dirty: false,
            history: History::default(),
        }
    }

    /// Get the content
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Get current cursor position (byte offset)
    pub fn cursor(&self) -> ByteOffset {
        self.cursor
    }

    /// Set cursor position (clamped to valid range)
    pub fn set_cursor(&mut self, pos: ByteOffset) {
        self.cursor = pos.min(ByteOffset(self.content.len()));
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
    pub fn insert_char(&mut self, ch: char) -> ByteOffset {
        // Ensure cursor is at a valid UTF-8 boundary
        let cursor = self.ensure_char_boundary(self.cursor);

        // Record for undo
        self.history.record(EditOperation::Insert {
            pos: cursor,
            text: ch.to_string(),
        });

        self.content.insert(cursor.get(), ch);
        self.cursor = cursor + ch.len_utf8();
        self.dirty = true;

        self.cursor
    }

    /// Insert a string at the cursor position
    /// Returns the new cursor position
    pub fn insert_str(&mut self, s: &str) -> ByteOffset {
        let cursor = self.ensure_char_boundary(self.cursor);

        // Record for undo
        if !s.is_empty() {
            self.history.record(EditOperation::Insert {
                pos: cursor,
                text: s.to_string(),
            });
        }

        self.content.insert_str(cursor.get(), s);
        self.cursor = cursor + s.len();
        self.dirty = true;

        self.cursor
    }

    /// Insert a newline at cursor position
    /// Returns the new cursor position
    pub fn insert_newline(&mut self) -> ByteOffset {
        self.insert_char('\n')
    }

    /// Delete the character before cursor (backspace)
    /// Returns true if a character was deleted
    pub fn delete_before(&mut self) -> bool {
        if self.cursor == ByteOffset::ZERO {
            return false;
        }

        // Find the start of the previous character by going back 1 byte first
        // then finding the char boundary (handles multi-byte UTF-8)
        let mut prev_char_start = self.cursor.get() - 1;
        while prev_char_start > 0 && !self.content.is_char_boundary(prev_char_start) {
            prev_char_start -= 1;
        }

        // Record for undo
        let deleted = self.content[prev_char_start..self.cursor.get()].to_string();
        self.history.record(EditOperation::Delete {
            pos: ByteOffset(prev_char_start),
            text: deleted,
        });

        // Remove the character
        self.content.drain(prev_char_start..self.cursor.get());
        self.cursor = ByteOffset(prev_char_start);
        self.dirty = true;

        true
    }

    /// Delete the character at cursor position (delete key)
    /// Returns true if a character was deleted
    pub fn delete_at(&mut self) -> bool {
        if self.cursor.get() >= self.content.len() {
            return false;
        }

        let cursor = self.ensure_char_boundary(self.cursor);
        let next_char_end = self.next_char_boundary(cursor.get());

        // Record for undo
        let deleted = self.content[cursor.get()..next_char_end].to_string();
        self.history.record(EditOperation::Delete {
            pos: cursor,
            text: deleted,
        });

        self.content.drain(cursor.get()..next_char_end);
        self.dirty = true;

        true
    }

    /// Delete a range of content
    pub fn delete_range(&mut self, start: ByteOffset, end: ByteOffset) {
        let start = self.ensure_char_boundary(start);
        let end = self.ensure_char_boundary(end.min(ByteOffset(self.content.len())));

        if start < end {
            // Record for undo
            let deleted = self.content[start.get()..end.get()].to_string();
            self.history.record(EditOperation::Delete {
                pos: start,
                text: deleted,
            });

            self.content.drain(start.get()..end.get());
            self.dirty = true;

            // Adjust cursor if it was in the deleted range
            if self.cursor > start {
                if self.cursor >= end {
                    self.cursor = self.cursor - (end - start);
                } else {
                    self.cursor = start;
                }
            }
        }
    }

    /// Replace all content (e.g., after reload)
    pub fn replace_content(&mut self, new_content: String) {
        self.content = new_content;
        self.cursor = self.cursor.min(ByteOffset(self.content.len()));
        // Don't mark dirty - this is a reload, not an edit
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor movement helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Move cursor to the next character
    pub fn move_cursor_forward(&mut self) {
        if self.cursor.get() < self.content.len() {
            self.cursor = ByteOffset(self.next_char_boundary(self.cursor.get()));
        }
    }

    /// Move cursor to the previous character
    pub fn move_cursor_back(&mut self) {
        if self.cursor.get() > 0 {
            self.cursor = ByteOffset(self.prev_char_boundary(self.cursor.get()));
        }
    }

    /// Move cursor to start of content
    pub fn move_cursor_to_start(&mut self) {
        self.cursor = ByteOffset::ZERO;
    }

    /// Move cursor to end of content
    pub fn move_cursor_to_end(&mut self) {
        self.cursor = ByteOffset(self.content.len());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // UTF-8 boundary helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Ensure position is at a valid UTF-8 character boundary
    fn ensure_char_boundary(&self, pos: ByteOffset) -> ByteOffset {
        let pos = pos.get().min(self.content.len());

        // If already at boundary, return as-is
        if self.content.is_char_boundary(pos) {
            return ByteOffset(pos);
        }

        // Search backward for the nearest boundary
        ByteOffset(self.prev_char_boundary(pos))
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

    // ─────────────────────────────────────────────────────────────────────────
    // Line-level helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Find the start of the line containing the given position
    pub fn line_start(&self, pos: ByteOffset) -> ByteOffset {
        let pos = pos.get().min(self.content.len());
        // Search backward for newline or start of content
        if let Some(idx) = self.content[..pos].rfind('\n') {
            ByteOffset(idx + 1) // Position after the newline
        } else {
            ByteOffset::ZERO // Start of content
        }
    }

    /// Find the end of the line containing the given position (before newline)
    pub fn line_end(&self, pos: ByteOffset) -> ByteOffset {
        let pos = pos.get().min(self.content.len());
        // Search forward for newline or end of content
        if let Some(idx) = self.content[pos..].find('\n') {
            ByteOffset(pos + idx)
        } else {
            ByteOffset(self.content.len())
        }
    }

    /// Get the content of the line containing the given position
    pub fn line_content(&self, pos: ByteOffset) -> &str {
        let start = self.line_start(pos);
        let end = self.line_end(pos);
        &self.content[start.get()..end.get()]
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Word-level navigation (for Ctrl+Arrow, Ctrl+Backspace, etc.)
    // ─────────────────────────────────────────────────────────────────────────

    /// Find the start of the current or previous word
    /// Words are defined as sequences of alphanumeric/underscore characters
    pub fn word_start(&self, pos: ByteOffset) -> ByteOffset {
        let pos = pos.get().min(self.content.len());
        if pos == 0 {
            return ByteOffset::ZERO;
        }

        let bytes = self.content.as_bytes();
        let mut p = pos;

        // Skip any whitespace before cursor
        while p > 0 && Self::is_whitespace_byte(bytes[p - 1]) {
            p -= 1;
        }

        // Skip the word (non-whitespace)
        while p > 0 && !Self::is_whitespace_byte(bytes[p - 1]) {
            p -= 1;
        }

        ByteOffset(p)
    }

    /// Find the end of the current or next word
    pub fn word_end(&self, pos: ByteOffset) -> ByteOffset {
        let pos = pos.get().min(self.content.len());
        if pos >= self.content.len() {
            return ByteOffset(self.content.len());
        }

        let bytes = self.content.as_bytes();
        let mut p = pos;

        // Skip any whitespace after cursor
        while p < bytes.len() && Self::is_whitespace_byte(bytes[p]) {
            p += 1;
        }

        // Skip the word (non-whitespace)
        while p < bytes.len() && !Self::is_whitespace_byte(bytes[p]) {
            p += 1;
        }

        ByteOffset(p)
    }

    /// Check if a byte is ASCII whitespace
    fn is_whitespace_byte(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | b'\n' | b'\r')
    }

    /// Delete from cursor to start of current/previous word (Ctrl+Backspace)
    /// Returns true if something was deleted
    pub fn delete_word_before(&mut self) -> bool {
        if self.cursor == ByteOffset::ZERO {
            return false;
        }

        let word_start = self.word_start(self.cursor);
        if word_start < self.cursor {
            // Record for undo
            let deleted = self.content[word_start.get()..self.cursor.get()].to_string();
            self.history.record(EditOperation::Delete {
                pos: word_start,
                text: deleted,
            });

            self.content.drain(word_start.get()..self.cursor.get());
            self.cursor = word_start;
            self.dirty = true;
            return true;
        }
        false
    }

    /// Delete from cursor to end of current/next word (Ctrl+Delete)
    /// Returns true if something was deleted
    pub fn delete_word_after(&mut self) -> bool {
        if self.cursor.get() >= self.content.len() {
            return false;
        }

        let word_end = self.word_end(self.cursor);
        if word_end > self.cursor {
            // Record for undo
            let deleted = self.content[self.cursor.get()..word_end.get()].to_string();
            self.history.record(EditOperation::Delete {
                pos: self.cursor,
                text: deleted,
            });

            self.content.drain(self.cursor.get()..word_end.get());
            self.dirty = true;
            return true;
        }
        false
    }

    /// Get the word boundaries at the given position (for double-click selection)
    /// Returns (start, end) of the word
    pub fn word_at(&self, pos: ByteOffset) -> (ByteOffset, ByteOffset) {
        let pos = pos.get().min(self.content.len());
        if pos >= self.content.len() {
            return (ByteOffset(pos), ByteOffset(pos));
        }

        let bytes = self.content.as_bytes();

        // Find word start
        let mut start = pos;
        while start > 0 && !Self::is_whitespace_byte(bytes[start - 1]) {
            start -= 1;
        }

        // Find word end
        let mut end = pos;
        while end < bytes.len() && !Self::is_whitespace_byte(bytes[end]) {
            end += 1;
        }

        (ByteOffset(start), ByteOffset(end))
    }

    /// Insert string at a specific position (not at cursor)
    pub fn insert_at(&mut self, pos: ByteOffset, s: &str) {
        let pos = self.ensure_char_boundary(pos).min(ByteOffset(self.content.len()));

        // Record for undo
        if !s.is_empty() {
            self.history.record(EditOperation::Insert {
                pos,
                text: s.to_string(),
            });
        }

        self.content.insert_str(pos.get(), s);
        self.dirty = true;

        // Adjust cursor if it was after insertion point
        if self.cursor >= pos {
            self.cursor = self.cursor + s.len();
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Undo/Redo operations
    // ─────────────────────────────────────────────────────────────────────────

    /// Undo the last edit operation
    /// Returns the cursor position to restore, or None if nothing to undo
    pub fn undo(&mut self) -> Option<ByteOffset> {
        if let Some(op) = self.history.pop_undo() {
            let cursor_pos = self.apply_operation_without_history(&op);
            self.dirty = true;
            Some(cursor_pos)
        } else {
            None
        }
    }

    /// Redo the last undone operation
    /// Returns the cursor position to restore, or None if nothing to redo
    pub fn redo(&mut self) -> Option<ByteOffset> {
        if let Some(op) = self.history.pop_redo() {
            let cursor_pos = self.apply_operation_without_history(&op);
            self.dirty = true;
            Some(cursor_pos)
        } else {
            None
        }
    }

    /// Apply an operation without recording it in history (used by undo/redo)
    fn apply_operation_without_history(&mut self, op: &EditOperation) -> ByteOffset {
        match op {
            EditOperation::Insert { pos, text } => {
                let pos = self.ensure_char_boundary(*pos).min(ByteOffset(self.content.len()));
                self.content.insert_str(pos.get(), text);
                pos + text.len() // Cursor at end of inserted text
            }
            EditOperation::Delete { pos, text } => {
                let pos = self.ensure_char_boundary(*pos);
                let end = (pos.get() + text.len()).min(self.content.len());
                self.content.drain(pos.get()..end);
                pos // Cursor at deletion point
            }
        }
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Count words in the content
    /// Uses simple whitespace splitting (matches most word processors)
    pub fn word_count(&self) -> usize {
        self.content.split_whitespace().count()
    }

    /// Count characters in the content (including whitespace)
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
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
        assert_eq!(editor.cursor(), ByteOffset::ZERO);
        assert!(!editor.is_dirty());
    }

    #[test]
    fn test_insert_char() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        editor.insert_char('!');
        assert_eq!(editor.content(), "Hello!");
        assert_eq!(editor.cursor(), ByteOffset(6));
        assert!(editor.is_dirty());
    }

    #[test]
    fn test_insert_char_middle() {
        let mut editor = EditorState::new("Hllo".to_string());
        editor.set_cursor(ByteOffset(1));
        editor.insert_char('e');
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), ByteOffset(2));
    }

    #[test]
    fn test_delete_before() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        assert!(editor.delete_before());
        assert_eq!(editor.content(), "Hell");
        assert_eq!(editor.cursor(), ByteOffset(4));
    }

    #[test]
    fn test_delete_before_at_start() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset::ZERO);
        assert!(!editor.delete_before());
        assert_eq!(editor.content(), "Hello");
    }

    #[test]
    fn test_delete_at() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset::ZERO);
        assert!(editor.delete_at());
        assert_eq!(editor.content(), "ello");
        assert_eq!(editor.cursor(), ByteOffset::ZERO);
    }

    #[test]
    fn test_delete_at_end() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        assert!(!editor.delete_at());
        assert_eq!(editor.content(), "Hello");
    }

    #[test]
    fn test_insert_newline() {
        let mut editor = EditorState::new("HelloWorld".to_string());
        editor.set_cursor(ByteOffset(5));
        editor.insert_newline();
        assert_eq!(editor.content(), "Hello\nWorld");
        assert_eq!(editor.cursor(), ByteOffset(6));
    }

    #[test]
    fn test_utf8_handling() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        editor.insert_char('世');  // Multi-byte character
        assert_eq!(editor.content(), "Hello世");
        assert_eq!(editor.cursor(), ByteOffset(8));  // 5 + 3 bytes for 世

        editor.delete_before();
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), ByteOffset(5));
    }

    #[test]
    fn test_delete_range() {
        let mut editor = EditorState::new("Hello World".to_string());
        editor.set_cursor(ByteOffset(8));  // Cursor at 'r'
        editor.delete_range(ByteOffset(5), ByteOffset(11));  // Delete " World"
        assert_eq!(editor.content(), "Hello");
        assert_eq!(editor.cursor(), ByteOffset(5));  // Cursor adjusted
    }

    #[test]
    fn test_line_boundaries() {
        let editor = EditorState::new("Hello\nWorld\nTest".to_string());

        // First line
        assert_eq!(editor.line_start(ByteOffset::ZERO), ByteOffset::ZERO);
        assert_eq!(editor.line_end(ByteOffset::ZERO), ByteOffset(5));
        assert_eq!(editor.line_content(ByteOffset::ZERO), "Hello");

        // Second line (cursor at 'W')
        assert_eq!(editor.line_start(ByteOffset(6)), ByteOffset(6));
        assert_eq!(editor.line_end(ByteOffset(6)), ByteOffset(11));
        assert_eq!(editor.line_content(ByteOffset(6)), "World");

        // Third line
        assert_eq!(editor.line_start(ByteOffset(12)), ByteOffset(12));
        assert_eq!(editor.line_end(ByteOffset(12)), ByteOffset(16));
        assert_eq!(editor.line_content(ByteOffset(12)), "Test");
    }

    #[test]
    fn test_insert_at() {
        let mut editor = EditorState::new("Hello World".to_string());
        editor.set_cursor(ByteOffset(6));  // Cursor at 'W'
        editor.insert_at(ByteOffset::ZERO, "## ");  // Insert at start
        assert_eq!(editor.content(), "## Hello World");
        assert_eq!(editor.cursor(), ByteOffset(9));  // Cursor adjusted (was after insertion point)

        // Insert at position after cursor
        let mut editor2 = EditorState::new("Hello World".to_string());
        editor2.set_cursor(ByteOffset::ZERO);  // Cursor at start
        editor2.insert_at(ByteOffset(5), "XX");  // Insert after cursor
        assert_eq!(editor2.content(), "HelloXX World");
        assert_eq!(editor2.cursor(), ByteOffset::ZERO);  // Cursor not adjusted
    }

    #[test]
    fn test_undo_insert() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        editor.insert_str(" World");
        assert_eq!(editor.content(), "Hello World");

        // Undo should restore original content
        let cursor = editor.undo();
        assert!(cursor.is_some());
        assert_eq!(editor.content(), "Hello");
    }

    #[test]
    fn test_undo_delete() {
        let mut editor = EditorState::new("Hello World".to_string());
        editor.set_cursor(ByteOffset(11));
        editor.delete_before(); // Delete 'd'
        assert_eq!(editor.content(), "Hello Worl");

        // Undo should restore deleted character
        let cursor = editor.undo();
        assert!(cursor.is_some());
        assert_eq!(editor.content(), "Hello World");
    }

    #[test]
    fn test_undo_redo() {
        let mut editor = EditorState::new("Hello".to_string());
        editor.set_cursor(ByteOffset(5));
        editor.insert_str("!");
        assert_eq!(editor.content(), "Hello!");

        // Undo
        editor.undo();
        assert_eq!(editor.content(), "Hello");

        // Redo
        editor.redo();
        assert_eq!(editor.content(), "Hello!");
    }

    #[test]
    fn test_multiple_undo() {
        let mut editor = EditorState::new("".to_string());
        editor.set_cursor(ByteOffset::ZERO);

        // Add some delay simulation by using separate insert operations
        editor.insert_str("A");
        std::thread::sleep(std::time::Duration::from_millis(600)); // Force no coalesce
        editor.insert_str("B");
        std::thread::sleep(std::time::Duration::from_millis(600));
        editor.insert_str("C");

        assert_eq!(editor.content(), "ABC");

        // Undo each operation
        editor.undo();
        assert_eq!(editor.content(), "AB");
        editor.undo();
        assert_eq!(editor.content(), "A");
        editor.undo();
        assert_eq!(editor.content(), "");
    }
}
