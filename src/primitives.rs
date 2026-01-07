//! Core primitives for type-safe coordinate handling
//!
//! This module introduces newtypes that prevent mixing up different
//! coordinate systems. The compiler catches bugs that would otherwise
//! be runtime errors.
//!
//! # Coordinate Systems
//!
//! - `ByteOffset`: Position in source markdown (byte index)
//! - `ScreenPos`: Position on rendered screen (line, column)
//! - `SourceSpan`: Range in source markdown
//!
//! # Migration Guide
//!
//! Replace `usize` cursor positions with `ByteOffset`:
//! ```rust
//! // Before:
//! cursor: usize
//!
//! // After:
//! cursor: ByteOffset
//! ```
//!
//! Replace `(usize, usize)` screen positions with `ScreenPos`:
//! ```rust
//! // Before:
//! fn source_to_screen(&self, offset: usize) -> Option<(usize, usize)>
//!
//! // After:
//! fn source_to_screen(&self, offset: ByteOffset) -> Option<ScreenPos>
//! ```

use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

// ═══════════════════════════════════════════════════════════════════════════
// BYTE OFFSET - Source Position Primitive
// ═══════════════════════════════════════════════════════════════════════════

/// Byte offset into the source markdown file
///
/// This is the universal coordinate system for editing operations.
/// All cursor positions, selections, and source spans use this type.
///
/// # Why a newtype?
///
/// Prevents accidentally passing a screen column where a byte offset
/// is expected. The type name documents intent at every call site.
///
/// # Examples
///
/// ```
/// let offset = ByteOffset::ZERO;
/// let next = offset + 5;  // Advance 5 bytes
/// let prev = next - 3;    // Retreat 3 bytes
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteOffset(pub usize);

impl ByteOffset {
    /// The start of the document
    pub const ZERO: ByteOffset = ByteOffset(0);

    /// Create a new byte offset
    #[inline]
    pub fn new(offset: usize) -> Self {
        ByteOffset(offset)
    }

    /// Get the raw value
    #[inline]
    pub fn get(self) -> usize {
        self.0
    }

    /// Advance by a number of bytes
    #[inline]
    pub fn advance(self, bytes: usize) -> Self {
        ByteOffset(self.0 + bytes)
    }

    /// Retreat by a number of bytes (saturating at 0)
    #[inline]
    pub fn retreat(self, bytes: usize) -> Self {
        ByteOffset(self.0.saturating_sub(bytes))
    }

    /// Clamp to a maximum value
    #[inline]
    pub fn min(self, other: ByteOffset) -> Self {
        ByteOffset(self.0.min(other.0))
    }

    /// Take the larger of two offsets
    #[inline]
    pub fn max(self, other: ByteOffset) -> Self {
        ByteOffset(self.0.max(other.0))
    }

    /// Distance between two offsets (absolute value)
    #[inline]
    pub fn distance(self, other: ByteOffset) -> usize {
        if self.0 > other.0 {
            self.0 - other.0
        } else {
            other.0 - self.0
        }
    }
}

// Arithmetic operations for ergonomic use
impl Add<usize> for ByteOffset {
    type Output = ByteOffset;

    #[inline]
    fn add(self, rhs: usize) -> ByteOffset {
        ByteOffset(self.0 + rhs)
    }
}

impl AddAssign<usize> for ByteOffset {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl Sub<usize> for ByteOffset {
    type Output = ByteOffset;

    #[inline]
    fn sub(self, rhs: usize) -> ByteOffset {
        ByteOffset(self.0.saturating_sub(rhs))
    }
}

impl SubAssign<usize> for ByteOffset {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 = self.0.saturating_sub(rhs);
    }
}

impl Sub<ByteOffset> for ByteOffset {
    type Output = usize;

    #[inline]
    fn sub(self, rhs: ByteOffset) -> usize {
        self.0.saturating_sub(rhs.0)
    }
}

impl From<usize> for ByteOffset {
    #[inline]
    fn from(value: usize) -> Self {
        ByteOffset(value)
    }
}

impl From<ByteOffset> for usize {
    #[inline]
    fn from(value: ByteOffset) -> Self {
        value.0
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SCREEN POSITION - Visual Position Primitive
// ═══════════════════════════════════════════════════════════════════════════

/// Position on the rendered screen (line, column)
///
/// Line 0 is the first rendered line. Column 0 is the leftmost position.
/// This is purely visual and doesn't account for scroll offset.
///
/// # Named Fields
///
/// Using named fields prevents (col, line) vs (line, col) bugs that
/// are common with tuple types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenPos {
    /// Line number (0-indexed from top of document)
    pub line: usize,
    /// Column number (0-indexed from left)
    pub col: usize,
}

impl ScreenPos {
    /// Create a new screen position
    #[inline]
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Position at the origin (0, 0)
    pub const ORIGIN: ScreenPos = ScreenPos { line: 0, col: 0 };

    /// Move to the next line, same column
    #[inline]
    pub fn next_line(self) -> Self {
        Self {
            line: self.line + 1,
            col: self.col,
        }
    }

    /// Move to the previous line, same column (saturating)
    #[inline]
    pub fn prev_line(self) -> Self {
        Self {
            line: self.line.saturating_sub(1),
            col: self.col,
        }
    }

    /// Check if within a viewport
    #[inline]
    pub fn is_visible(&self, scroll: usize, viewport_height: usize) -> bool {
        self.line >= scroll && self.line < scroll + viewport_height
    }

    /// Convert to screen coordinates accounting for scroll
    #[inline]
    pub fn to_screen_y(&self, scroll: usize) -> Option<usize> {
        if self.line >= scroll {
            Some(self.line - scroll)
        } else {
            None
        }
    }
}

impl From<(usize, usize)> for ScreenPos {
    #[inline]
    fn from((line, col): (usize, usize)) -> Self {
        ScreenPos { line, col }
    }
}

impl From<ScreenPos> for (usize, usize) {
    #[inline]
    fn from(pos: ScreenPos) -> Self {
        (pos.line, pos.col)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SOURCE SPAN - Range in Source (upgraded version)
// ═══════════════════════════════════════════════════════════════════════════

/// Byte offset range in source markdown
///
/// This is the type-safe version of `Range<usize>` for source positions.
/// Using `ByteOffset` prevents confusion with screen ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceSpan {
    /// Start byte offset (inclusive)
    pub start: ByteOffset,
    /// End byte offset (exclusive)
    pub end: ByteOffset,
}

impl SourceSpan {
    /// Create a new source span
    #[inline]
    pub fn new(start: ByteOffset, end: ByteOffset) -> Self {
        Self { start, end }
    }

    /// Create from raw usize values (for migration)
    #[inline]
    pub fn from_usize(start: usize, end: usize) -> Self {
        Self {
            start: ByteOffset(start),
            end: ByteOffset(end),
        }
    }

    /// Create from a std::ops::Range
    #[inline]
    pub fn from_range(range: Range<usize>) -> Self {
        Self {
            start: ByteOffset(range.start),
            end: ByteOffset(range.end),
        }
    }

    /// Check if an offset is within this span
    #[inline]
    pub fn contains(&self, offset: ByteOffset) -> bool {
        offset >= self.start && offset < self.end
    }

    /// Length of the span in bytes
    #[inline]
    pub fn len(&self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Check if the span is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Convert to std::ops::Range
    #[inline]
    pub fn to_range(&self) -> Range<usize> {
        self.start.0..self.end.0
    }

    /// Extend the span to include another span
    #[inline]
    pub fn extend(&mut self, other: SourceSpan) {
        self.start = self.start.min(other.start);
        self.end = self.end.max(other.end);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_offset_arithmetic() {
        let offset = ByteOffset::ZERO;
        assert_eq!(offset + 5, ByteOffset(5));
        assert_eq!(ByteOffset(10) - 3, ByteOffset(7));
        assert_eq!(ByteOffset(3) - 5, ByteOffset(0)); // Saturating
    }

    #[test]
    fn test_byte_offset_distance() {
        assert_eq!(ByteOffset(10).distance(ByteOffset(3)), 7);
        assert_eq!(ByteOffset(3).distance(ByteOffset(10)), 7);
    }

    #[test]
    fn test_screen_pos() {
        let pos = ScreenPos::new(5, 10);
        assert!(pos.is_visible(3, 5)); // Lines 3-7 visible
        assert!(!pos.is_visible(6, 5)); // Lines 6-10 visible

        assert_eq!(pos.to_screen_y(3), Some(2)); // Line 5 is at screen y=2
        assert_eq!(pos.to_screen_y(6), None); // Line 5 is above viewport
    }

    #[test]
    fn test_source_span() {
        let span = SourceSpan::from_usize(10, 20);
        assert_eq!(span.len(), 10);
        assert!(span.contains(ByteOffset(10)));
        assert!(span.contains(ByteOffset(19)));
        assert!(!span.contains(ByteOffset(20)));
        assert!(!span.contains(ByteOffset(9)));
    }

    #[test]
    fn test_screen_pos_conversion() {
        let pos = ScreenPos::new(5, 10);
        let tuple: (usize, usize) = pos.into();
        assert_eq!(tuple, (5, 10));

        let back: ScreenPos = tuple.into();
        assert_eq!(back, pos);
    }
}
