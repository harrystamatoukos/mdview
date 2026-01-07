//! Cursor state for WYSIWYG editing
//!
//! Encapsulates cursor position and visibility state,
//! separated from the main Pager struct for cleaner organization.

use crate::primitives::{ByteOffset, ScreenPos};

/// Cursor state for text editing
///
/// Tracks the cursor position in source bytes and visibility state
/// for blinking. Also handles visual overrides for special cases
/// like positioning after Enter key.
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Current cursor position as source byte offset (None = no cursor)
    position: Option<ByteOffset>,
    /// Whether cursor is currently visible (for blinking)
    visible: bool,
    /// Visual position override - used after Enter to position cursor
    /// where content WILL appear. Cleared on first keystroke when
    /// actual content exists to map to.
    visual_override: Option<ScreenPos>,
}

impl CursorState {
    /// Create a new cursor state at the given position
    pub fn new(position: Option<ByteOffset>) -> Self {
        Self {
            position,
            visible: true,
            visual_override: None,
        }
    }

    /// Get the current cursor position
    pub fn position(&self) -> Option<ByteOffset> {
        self.position
    }

    /// Set the cursor position
    pub fn set_position(&mut self, position: ByteOffset) {
        self.position = Some(position);
    }

    /// Set the cursor position (optional variant)
    pub fn set_position_opt(&mut self, position: Option<ByteOffset>) {
        self.position = position;
    }

    /// Check if cursor is visible (for blinking)
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Toggle cursor visibility (for blinking)
    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    /// Get the visual override position if set
    pub fn visual_override(&self) -> Option<ScreenPos> {
        self.visual_override
    }

    /// Set a visual position override
    pub fn set_visual_override(&mut self, line: usize, col: usize) {
        self.visual_override = Some(ScreenPos::new(line, col));
    }

    /// Set a visual position override from a ScreenPos
    pub fn set_visual_override_pos(&mut self, pos: ScreenPos) {
        self.visual_override = Some(pos);
    }

    /// Clear the visual override
    pub fn clear_visual_override(&mut self) {
        self.visual_override = None;
    }

    /// Check if there is a visual override set
    pub fn has_visual_override(&self) -> bool {
        self.visual_override.is_some()
    }
}

impl Default for CursorState {
    fn default() -> Self {
        Self::new(None)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cursor_state() {
        let cursor = CursorState::new(Some(ByteOffset(10)));
        assert_eq!(cursor.position(), Some(ByteOffset(10)));
        assert!(cursor.is_visible());
        assert!(cursor.visual_override().is_none());
    }

    #[test]
    fn test_new_cursor_state_none() {
        let cursor = CursorState::new(None);
        assert_eq!(cursor.position(), None);
    }

    #[test]
    fn test_set_position() {
        let mut cursor = CursorState::new(None);
        cursor.set_position(ByteOffset(42));
        assert_eq!(cursor.position(), Some(ByteOffset(42)));
    }

    #[test]
    fn test_toggle_visibility() {
        let mut cursor = CursorState::new(None);
        assert!(cursor.is_visible());

        cursor.toggle_visibility();
        assert!(!cursor.is_visible());

        cursor.toggle_visibility();
        assert!(cursor.is_visible());
    }

    #[test]
    fn test_visual_override() {
        let mut cursor = CursorState::new(None);
        assert!(!cursor.has_visual_override());
        assert!(cursor.visual_override().is_none());

        cursor.set_visual_override(5, 10);
        assert!(cursor.has_visual_override());
        assert_eq!(cursor.visual_override(), Some(ScreenPos::new(5, 10)));

        cursor.clear_visual_override();
        assert!(!cursor.has_visual_override());
        assert!(cursor.visual_override().is_none());
    }

    #[test]
    fn test_default() {
        let cursor = CursorState::default();
        assert_eq!(cursor.position(), None);
        assert!(cursor.is_visible());
    }
}
