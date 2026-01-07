//! Selection state for WYSIWYG editing
//!
//! This module manages text selection, tracking the anchor point (where
//! selection started) and cursor point (current selection end).

use crate::primitives::ByteOffset;

/// Text selection represented as two byte offsets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where selection started (anchor point)
    pub anchor: ByteOffset,
    /// Current end of selection (cursor point)
    pub cursor: ByteOffset,
}

impl Selection {
    /// Create a new selection at a single point (no text selected)
    pub fn new(position: ByteOffset) -> Self {
        Self {
            anchor: position,
            cursor: position,
        }
    }

    /// Create a selection spanning from anchor to cursor
    pub fn from_range(anchor: ByteOffset, cursor: ByteOffset) -> Self {
        Self { anchor, cursor }
    }

    /// Check if there's an active selection (anchor != cursor)
    pub fn is_active(&self) -> bool {
        self.anchor != self.cursor
    }

    /// Get the selection as a normalized range (start, end) where start <= end
    pub fn range(&self) -> (ByteOffset, ByteOffset) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// Get the start of selection (smaller offset)
    pub fn start(&self) -> ByteOffset {
        self.anchor.min(self.cursor)
    }

    /// Get the end of selection (larger offset)
    pub fn end(&self) -> ByteOffset {
        self.anchor.max(self.cursor)
    }

    /// Get the length of selection in bytes
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// Check if selection is empty (no text selected)
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }

    /// Move cursor while keeping anchor fixed (extends/shrinks selection)
    pub fn extend_to(&mut self, position: ByteOffset) {
        self.cursor = position;
    }

    /// Collapse selection to cursor position
    pub fn collapse_to_cursor(&mut self) {
        self.anchor = self.cursor;
    }

    /// Collapse selection to anchor position
    pub fn collapse_to_anchor(&mut self) {
        self.cursor = self.anchor;
    }

    /// Collapse selection to start (smaller offset)
    pub fn collapse_to_start(&mut self) {
        let start = self.start();
        self.anchor = start;
        self.cursor = start;
    }

    /// Collapse selection to end (larger offset)
    pub fn collapse_to_end(&mut self) {
        let end = self.end();
        self.anchor = end;
        self.cursor = end;
    }

    /// Set both anchor and cursor to the same position (move without selecting)
    pub fn move_to(&mut self, position: ByteOffset) {
        self.anchor = position;
        self.cursor = position;
    }

    /// Check if a byte offset is within the selection
    pub fn contains(&self, offset: ByteOffset) -> bool {
        let (start, end) = self.range();
        offset >= start && offset < end
    }

    /// Select all (from start to end of document)
    pub fn select_all(&mut self, document_len: usize) {
        self.anchor = ByteOffset::ZERO;
        self.cursor = ByteOffset(document_len);
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::new(ByteOffset::ZERO)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_selection() {
        let sel = Selection::new(ByteOffset(10));
        assert_eq!(sel.anchor, ByteOffset(10));
        assert_eq!(sel.cursor, ByteOffset(10));
        assert!(!sel.is_active());
        assert!(sel.is_empty());
    }

    #[test]
    fn test_active_selection() {
        let sel = Selection::from_range(ByteOffset(5), ByteOffset(15));
        assert!(sel.is_active());
        assert!(!sel.is_empty());
        assert_eq!(sel.start(), ByteOffset(5));
        assert_eq!(sel.end(), ByteOffset(15));
        assert_eq!(sel.len(), 10);
    }

    #[test]
    fn test_reversed_selection() {
        // Selecting backwards (cursor before anchor)
        let sel = Selection::from_range(ByteOffset(15), ByteOffset(5));
        assert!(sel.is_active());
        assert_eq!(sel.start(), ByteOffset(5));  // start is always smaller
        assert_eq!(sel.end(), ByteOffset(15));   // end is always larger
        assert_eq!(sel.range(), (ByteOffset(5), ByteOffset(15)));
    }

    #[test]
    fn test_extend_selection() {
        let mut sel = Selection::new(ByteOffset(10));
        sel.extend_to(ByteOffset(20));
        assert!(sel.is_active());
        assert_eq!(sel.anchor, ByteOffset(10));
        assert_eq!(sel.cursor, ByteOffset(20));
    }

    #[test]
    fn test_collapse_selection() {
        let mut sel = Selection::from_range(ByteOffset(10), ByteOffset(20));

        sel.collapse_to_cursor();
        assert!(!sel.is_active());
        assert_eq!(sel.anchor, ByteOffset(20));
        assert_eq!(sel.cursor, ByteOffset(20));

        let mut sel = Selection::from_range(ByteOffset(10), ByteOffset(20));
        sel.collapse_to_start();
        assert_eq!(sel.anchor, ByteOffset(10));
        assert_eq!(sel.cursor, ByteOffset(10));

        let mut sel = Selection::from_range(ByteOffset(10), ByteOffset(20));
        sel.collapse_to_end();
        assert_eq!(sel.anchor, ByteOffset(20));
        assert_eq!(sel.cursor, ByteOffset(20));
    }

    #[test]
    fn test_contains() {
        let sel = Selection::from_range(ByteOffset(10), ByteOffset(20));
        assert!(!sel.contains(ByteOffset(9)));
        assert!(sel.contains(ByteOffset(10)));
        assert!(sel.contains(ByteOffset(15)));
        assert!(sel.contains(ByteOffset(19)));
        assert!(!sel.contains(ByteOffset(20)));  // end is exclusive
    }

    #[test]
    fn test_move_to() {
        let mut sel = Selection::from_range(ByteOffset(10), ByteOffset(20));
        sel.move_to(ByteOffset(30));
        assert!(!sel.is_active());
        assert_eq!(sel.anchor, ByteOffset(30));
        assert_eq!(sel.cursor, ByteOffset(30));
    }

    #[test]
    fn test_select_all() {
        let mut sel = Selection::new(ByteOffset(50));
        sel.select_all(100);
        assert_eq!(sel.anchor, ByteOffset::ZERO);
        assert_eq!(sel.cursor, ByteOffset(100));
        assert_eq!(sel.len(), 100);
    }
}
