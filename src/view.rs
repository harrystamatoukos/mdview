//! View state for the pager
//!
//! Encapsulates scroll position and animation state, separated from
//! the main Pager struct for cleaner organization.

// Easing factor: higher = snappier, lower = smoother.
// Terminal scrolling is line-based, so a slow easing value feels like input lag.
const SCROLL_EASING: f64 = 0.55;
// Threshold to snap to target (avoid endless micro-animations)
const SCROLL_SNAP_THRESHOLD: f64 = 0.25;
// Keep repeated wheel/trackpad events from pushing the visual position far
// behind the target. This preserves smoothing without a sluggish tail.
const MAX_SCROLL_LAG: f64 = 6.0;

/// Viewport and scroll state
///
/// Manages smooth scrolling animation with exponential easing.
/// The scroll position is in fractional lines for smooth animation.
#[derive(Debug, Clone)]
pub struct ViewState {
    /// Target scroll position (where we want to scroll to)
    scroll_target: f64,
    /// Current animated scroll position
    scroll_current: f64,
}

impl ViewState {
    /// Create a new view state starting at the top
    pub fn new() -> Self {
        Self {
            scroll_target: 0.0,
            scroll_current: 0.0,
        }
    }

    /// Scroll up by a number of lines
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_target = (self.scroll_target - lines as f64).max(0.0);
        self.limit_lag();
    }

    /// Scroll down by a number of lines
    ///
    /// `total_lines` is the total number of content lines
    /// `viewport_height` is the visible area height in lines
    pub fn scroll_down(&mut self, lines: usize, total_lines: usize, viewport_height: usize) {
        let max_scroll = total_lines.saturating_sub(viewport_height) as f64;
        self.scroll_target = (self.scroll_target + lines as f64).min(max_scroll);
        self.limit_lag();
    }

    /// Jump to the top of the document
    pub fn scroll_to_top(&mut self) {
        self.scroll_target = 0.0;
        self.limit_lag();
    }

    /// Jump to the bottom of the document
    pub fn scroll_to_bottom(&mut self, total_lines: usize, viewport_height: usize) {
        self.scroll_target = total_lines.saturating_sub(viewport_height) as f64;
        self.limit_lag();
    }

    /// Clamp current and target scroll positions to the available content range.
    pub fn clamp_to_content(&mut self, total_lines: usize, viewport_height: usize) {
        let max_scroll = total_lines.saturating_sub(viewport_height) as f64;
        self.scroll_target = self.scroll_target.clamp(0.0, max_scroll);
        self.scroll_current = self.scroll_current.clamp(0.0, max_scroll);
    }

    /// Scroll to ensure a specific line is visible
    ///
    /// `line` is the line number to make visible
    /// `viewport_height` is the visible area height
    /// `margin` is the number of lines to keep above/below the target
    #[allow(dead_code)] // scroll API exercised by tests
    pub fn ensure_line_visible(&mut self, line: usize, viewport_height: usize, margin: usize) {
        if viewport_height == 0 {
            self.scroll_target = line as f64;
            self.limit_lag();
            return;
        }

        let margin = margin.min(viewport_height.saturating_sub(1));
        let scroll_pos = self.position();

        // Scroll up if line is above viewport (with margin)
        if line < scroll_pos + margin {
            self.scroll_target = line.saturating_sub(margin) as f64;
        }
        // Scroll down if line is below viewport (with margin)
        else if line >= scroll_pos + viewport_height.saturating_sub(margin) {
            let bottom_margin = viewport_height.saturating_sub(margin + 1);
            self.scroll_target = line.saturating_sub(bottom_margin) as f64;
        }
        self.limit_lag();
    }

    fn limit_lag(&mut self) {
        let diff = self.scroll_target - self.scroll_current;
        if diff > MAX_SCROLL_LAG {
            self.scroll_current = self.scroll_target - MAX_SCROLL_LAG;
        } else if diff < -MAX_SCROLL_LAG {
            self.scroll_current = self.scroll_target + MAX_SCROLL_LAG;
        }
    }

    /// Update animation state
    ///
    /// Returns `true` if still animating (needs another frame),
    /// `false` if animation is complete.
    pub fn update_animation(&mut self) -> bool {
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

    /// Get the current scroll position for rendering (integer line number)
    pub fn position(&self) -> usize {
        self.scroll_current.round() as usize
    }

    /// Get the target scroll position
    #[allow(dead_code)] // exercised by tests
    pub fn target(&self) -> f64 {
        self.scroll_target
    }

    /// Check if currently animating
    #[allow(dead_code)] // exercised by tests
    pub fn is_animating(&self) -> bool {
        (self.scroll_target - self.scroll_current).abs() >= SCROLL_SNAP_THRESHOLD
    }
}

impl Default for ViewState {
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
    fn test_new_view_state() {
        let view = ViewState::new();
        assert_eq!(view.position(), 0);
        assert_eq!(view.target(), 0.0);
        assert!(!view.is_animating());
    }

    #[test]
    fn test_scroll_up() {
        let mut view = ViewState::new();
        view.scroll_target = 10.0;
        view.scroll_current = 10.0;

        view.scroll_up(3);
        assert_eq!(view.target(), 7.0);

        // Can't scroll above 0
        view.scroll_up(20);
        assert_eq!(view.target(), 0.0);
    }

    #[test]
    fn test_scroll_down() {
        let mut view = ViewState::new();
        let total_lines = 100;
        let viewport = 20;

        view.scroll_down(10, total_lines, viewport);
        assert_eq!(view.target(), 10.0);

        // Can't scroll past max
        view.scroll_down(100, total_lines, viewport);
        assert_eq!(view.target(), 80.0); // max = 100 - 20
    }

    #[test]
    fn test_animation() {
        let mut view = ViewState::new();
        view.scroll_target = 10.0;

        // Should be animating
        assert!(view.is_animating());

        // Run animation until complete
        let mut iterations = 0;
        while view.update_animation() && iterations < 100 {
            iterations += 1;
        }

        // Should have reached target
        assert_eq!(view.position(), 10);
        assert!(!view.is_animating());
    }

    #[test]
    fn test_ensure_line_visible() {
        let mut view = ViewState::new();
        view.scroll_current = 50.0;
        view.scroll_target = 50.0;

        let viewport = 20;
        let margin = 3;

        // Line within viewport - no scroll needed
        view.ensure_line_visible(60, viewport, margin);
        assert_eq!(view.target(), 50.0);

        // Line above viewport - scroll up
        view.ensure_line_visible(40, viewport, margin);
        assert!(view.target() < 50.0);

        // Line below viewport - scroll down
        view.scroll_target = 50.0;
        view.ensure_line_visible(75, viewport, margin);
        assert!(view.target() > 50.0);
    }

    #[test]
    fn test_clamp_to_content() {
        let mut view = ViewState::new();
        view.scroll_current = 100.0;
        view.scroll_target = 100.0;

        view.clamp_to_content(30, 10);

        assert_eq!(view.position(), 20);
        assert_eq!(view.target(), 20.0);
    }

    #[test]
    fn test_ensure_line_visible_zero_viewport() {
        let mut view = ViewState::new();

        view.ensure_line_visible(5, 0, 3);

        assert_eq!(view.target(), 5.0);
    }
}
