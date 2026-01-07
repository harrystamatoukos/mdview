//! Input state for mouse and click handling
//!
//! Encapsulates mouse drag state and multi-click detection,
//! separated from the main Pager struct for cleaner organization.

use std::time::Instant;

/// Click type detected from timing and position
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickType {
    Single,
    Double,
    Triple,
}

/// Input state for mouse handling
///
/// Tracks mouse drag state and detects double/triple clicks
/// based on timing and position proximity.
#[derive(Debug, Clone)]
pub struct InputState {
    /// Whether mouse is currently being dragged
    dragging: bool,
    /// Time of last click (for multi-click detection)
    last_click_time: Instant,
    /// Position of last click (for multi-click detection)
    last_click_pos: (u16, u16),
    /// Current click count in sequence (1, 2, or 3)
    click_count: u8,
    /// Last input time (for responsive polling)
    last_input_time: Instant,
}

// Multi-click detection threshold (ms)
const MULTI_CLICK_TIMEOUT_MS: u128 = 400;
// Position tolerance for multi-click (pixels)
const MULTI_CLICK_TOLERANCE: i32 = 2;
// Fast polling duration after input (ms)
const FAST_POLL_DURATION_MS: u128 = 1000;

impl InputState {
    /// Create a new input state
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            dragging: false,
            last_click_time: now,
            last_click_pos: (0, 0),
            click_count: 0,
            last_input_time: now,
        }
    }

    /// Record a click and return the click type (single/double/triple)
    ///
    /// Call this when a mouse click is detected. The method tracks
    /// timing and position to detect multi-clicks.
    pub fn record_click(&mut self, x: u16, y: u16) -> ClickType {
        let now = Instant::now();

        // Check if this is part of a multi-click sequence
        let is_same_pos = (x as i32 - self.last_click_pos.0 as i32).abs() <= MULTI_CLICK_TOLERANCE
            && (y as i32 - self.last_click_pos.1 as i32).abs() <= MULTI_CLICK_TOLERANCE;
        let is_quick = now.duration_since(self.last_click_time).as_millis() < MULTI_CLICK_TIMEOUT_MS;

        if is_same_pos && is_quick {
            // Continue multi-click sequence (wraps: 1 -> 2 -> 3 -> 1)
            self.click_count = (self.click_count % 3) + 1;
        } else {
            // Start new click sequence
            self.click_count = 1;
        }

        self.last_click_time = now;
        self.last_click_pos = (x, y);
        self.last_input_time = now;

        match self.click_count {
            1 => ClickType::Single,
            2 => ClickType::Double,
            _ => ClickType::Triple,
        }
    }

    /// Start a mouse drag operation
    pub fn start_drag(&mut self) {
        self.dragging = true;
        self.mark_input();
    }

    /// End a mouse drag operation
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// Check if currently dragging
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Mark that input occurred (for responsive polling)
    pub fn mark_input(&mut self) {
        self.last_input_time = Instant::now();
    }

    /// Check if we need fast polling (recent input activity)
    pub fn needs_fast_poll(&self) -> bool {
        self.last_input_time.elapsed().as_millis() < FAST_POLL_DURATION_MS
    }

    /// Get the current click count (1, 2, or 3)
    pub fn click_count(&self) -> u8 {
        self.click_count
    }
}

impl Default for InputState {
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
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_new_input_state() {
        let input = InputState::new();
        assert!(!input.is_dragging());
        assert_eq!(input.click_count(), 0);
    }

    #[test]
    fn test_single_click() {
        let mut input = InputState::new();
        let click_type = input.record_click(100, 100);
        assert_eq!(click_type, ClickType::Single);
        assert_eq!(input.click_count(), 1);
    }

    #[test]
    fn test_double_click() {
        let mut input = InputState::new();

        // First click
        input.record_click(100, 100);

        // Second click at same position (quick)
        let click_type = input.record_click(100, 100);
        assert_eq!(click_type, ClickType::Double);
        assert_eq!(input.click_count(), 2);
    }

    #[test]
    fn test_triple_click() {
        let mut input = InputState::new();

        // Three quick clicks at same position
        input.record_click(100, 100);
        input.record_click(100, 100);
        let click_type = input.record_click(100, 100);

        assert_eq!(click_type, ClickType::Triple);
        assert_eq!(input.click_count(), 3);
    }

    #[test]
    fn test_click_sequence_wraps() {
        let mut input = InputState::new();

        // Four quick clicks should wrap back to single
        input.record_click(100, 100);
        input.record_click(100, 100);
        input.record_click(100, 100);
        let click_type = input.record_click(100, 100);

        assert_eq!(click_type, ClickType::Single);
        assert_eq!(input.click_count(), 1);
    }

    #[test]
    fn test_click_different_position_resets() {
        let mut input = InputState::new();

        // First click
        input.record_click(100, 100);

        // Second click at different position
        let click_type = input.record_click(200, 200);
        assert_eq!(click_type, ClickType::Single);
        assert_eq!(input.click_count(), 1);
    }

    #[test]
    fn test_click_timeout_resets() {
        let mut input = InputState::new();

        // First click
        input.record_click(100, 100);

        // Wait longer than timeout
        sleep(Duration::from_millis(500));

        // Second click (should be single due to timeout)
        let click_type = input.record_click(100, 100);
        assert_eq!(click_type, ClickType::Single);
    }

    #[test]
    fn test_drag_state() {
        let mut input = InputState::new();

        assert!(!input.is_dragging());

        input.start_drag();
        assert!(input.is_dragging());

        input.end_drag();
        assert!(!input.is_dragging());
    }

    #[test]
    fn test_fast_poll() {
        let mut input = InputState::new();

        // Should need fast poll right after creation
        assert!(input.needs_fast_poll());

        input.mark_input();
        assert!(input.needs_fast_poll());
    }
}
