//! Cursor display state for WYSIWYG editing
//!
//! Encapsulates cursor visibility and visual override state,
//! separated from position which is tracked by EditorState.
//!
//! # Position Source of Truth
//!
//! The cursor position is stored in `EditorState.cursor` - this module
//! only tracks display concerns:
//! - `visible`: For cursor blinking animation
//! - `visual_override`: For special positioning (e.g., after Enter key)

use crate::primitives::ScreenPos;

/// Cursor display state for text editing
///
/// This struct handles display concerns only. The actual cursor position
/// is managed by `EditorState`. Use `EditorState::cursor()` to get position.
///
/// Visual overrides are used for cases where the cursor should appear
/// at a position that doesn't yet have corresponding source content
/// (e.g., after pressing Enter before any text is typed).
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Whether cursor is currently visible (for blinking)
    visible: bool,
    /// Visual position override - used after Enter to position cursor
    /// where content WILL appear. Cleared on first keystroke when
    /// actual content exists to map to.
    visual_override: Option<ScreenPos>,
}

impl CursorState {
    /// Create a new cursor display state
    pub fn new() -> Self {
        Self {
            visible: true,
            visual_override: None,
        }
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
        Self::new()
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
        let cursor = CursorState::new();
        assert!(cursor.is_visible());
        assert!(cursor.visual_override().is_none());
    }

    #[test]
    fn test_toggle_visibility() {
        let mut cursor = CursorState::new();
        assert!(cursor.is_visible());

        cursor.toggle_visibility();
        assert!(!cursor.is_visible());

        cursor.toggle_visibility();
        assert!(cursor.is_visible());
    }

    #[test]
    fn test_visual_override() {
        let mut cursor = CursorState::new();
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
        assert!(cursor.is_visible());
        assert!(!cursor.has_visual_override());
    }
}
