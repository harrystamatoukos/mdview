//! Position mapping for WYSIWYG editing
//!
//! This module provides bidirectional mapping between:
//! - Source positions (byte offsets in the markdown file)
//! - Screen positions (line, column in the rendered view)
//!
//! The key insight: every visible character in the rendered view
//! either maps to a source byte offset, or is "synthetic" (margins,
//! borders, list bullets) with no source position.

use std::ops::Range;

// ═══════════════════════════════════════════════════════════════════════════
// SOURCE SPAN - Range in the original markdown
// ═══════════════════════════════════════════════════════════════════════════

/// Byte offset range in source markdown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceSpan {
    /// Start byte offset (inclusive)
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
}

impl SourceSpan {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn from_range(range: Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }

    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub fn to_range(&self) -> Range<usize> {
        self.start..self.end
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MAPPED CHARACTER - Single rendered character with source info
// ═══════════════════════════════════════════════════════════════════════════

/// A character in the rendered view with its source mapping
#[derive(Debug, Clone)]
pub struct MappedChar {
    /// The visible character
    pub ch: char,
    /// Source byte offset, or None for synthetic characters
    /// (margins, borders, list bullets, etc.)
    pub source_offset: Option<usize>,
}

impl MappedChar {
    /// Create a mapped character with a source position
    pub fn with_source(ch: char, offset: usize) -> Self {
        Self {
            ch,
            source_offset: Some(offset),
        }
    }

    /// Create a synthetic character (no source position)
    pub fn synthetic(ch: char) -> Self {
        Self {
            ch,
            source_offset: None,
        }
    }

    /// Check if this is a synthetic (non-source) character
    pub fn is_synthetic(&self) -> bool {
        self.source_offset.is_none()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LAYOUT MAP - Bidirectional screen ↔ source mapping
// ═══════════════════════════════════════════════════════════════════════════

/// Maps between rendered screen positions and source byte offsets
///
/// The LayoutMap is built during rendering and provides O(log n) lookups
/// in both directions using a sorted index with binary search.
#[derive(Debug, Clone)]
pub struct LayoutMap {
    /// For each rendered line: Vec of MappedChars
    lines: Vec<Vec<MappedChar>>,
    /// Sorted index for fast lookups: (source_offset, line, col)
    offset_index: Vec<(usize, usize, usize)>,
    /// Whether the index has been built
    index_built: bool,
}

impl LayoutMap {
    /// Create a new empty layout map
    #[inline]
    pub fn new(_source_len: usize) -> Self {
        Self {
            lines: Vec::new(),
            offset_index: Vec::new(),
            index_built: false,
        }
    }

    /// Add a rendered line to the map
    pub fn push_line(&mut self, chars: Vec<MappedChar>) {
        self.lines.push(chars);
        self.index_built = false; // Invalidate index
    }

    /// Get the number of rendered lines
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Get a line by index
    #[inline]
    pub fn line(&self, line: usize) -> Option<&[MappedChar]> {
        self.lines.get(line).map(|v| v.as_slice())
    }

    /// Build the offset index for fast source-to-screen lookups
    /// Call this after all lines have been added
    pub fn build_index(&mut self) {
        if self.index_built {
            return;
        }

        // Pre-allocate: estimate ~1 mapped char per 2 screen chars + end positions
        let estimated_size: usize = self.lines.iter().map(|l| l.len() / 2 + 1).sum();
        self.offset_index = Vec::with_capacity(estimated_size);

        for (line_idx, line) in self.lines.iter().enumerate() {
            let mut last_on_line: Option<(usize, usize, char)> = None;

            for (col_idx, mc) in line.iter().enumerate() {
                if let Some(offset) = mc.source_offset {
                    self.offset_index.push((offset, line_idx, col_idx));
                    last_on_line = Some((offset, col_idx, mc.ch));
                }
            }

            // Add end-of-line position for cursor placement after last char
            if let Some((offset, col, ch)) = last_on_line {
                self.offset_index.push((offset + ch.len_utf8(), line_idx, col + 1));
            }
        }

        // Unstable sort is faster and we don't need stability
        self.offset_index.sort_unstable_by_key(|(offset, _, _)| *offset);
        self.offset_index.dedup_by_key(|(offset, _, _)| *offset);
        self.index_built = true;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Screen → Source mapping
    // ─────────────────────────────────────────────────────────────────────────

    /// Get source offset for a screen position
    /// Returns None if position is out of bounds or on a synthetic character
    pub fn screen_to_source(&self, line: usize, col: usize) -> Option<usize> {
        self.lines
            .get(line)
            .and_then(|l| l.get(col))
            .and_then(|mc| mc.source_offset)
    }

    /// Get source offset for screen position, finding nearest if exact not available
    #[inline]
    pub fn screen_to_source_nearest(&self, line: usize, col: usize) -> Option<usize> {
        let line_chars = self.lines.get(line)?;

        // Try exact position first (most common case)
        if let Some(mc) = line_chars.get(col) {
            if let Some(offset) = mc.source_offset {
                return Some(offset);
            }
        }

        // Clicking past end of line - find last mapped char
        if col >= line_chars.len() {
            return line_chars.iter().rev()
                .find_map(|mc| mc.source_offset.map(|o| o + mc.ch.len_utf8()));
        }

        // Search outward from click position - left first (more natural for text)
        line_chars[..col].iter().rev()
            .find_map(|mc| mc.source_offset)
            .or_else(|| {
                line_chars[col + 1..].iter()
                    .find_map(|mc| mc.source_offset)
            })
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Source → Screen mapping
    // ─────────────────────────────────────────────────────────────────────────

    /// Get screen position for a source offset
    /// Uses binary search for O(log n) lookup (must call build_index first)
    #[inline]
    pub fn source_to_screen(&self, offset: usize) -> Option<(usize, usize)> {
        if !self.index_built || self.offset_index.is_empty() {
            return self.source_to_screen_linear(offset);
        }

        // End position is now in the index, so standard binary search works
        match self.offset_index.binary_search_by_key(&offset, |(o, _, _)| *o) {
            Ok(idx) => {
                let (_, line, col) = self.offset_index[idx];
                Some((line, col))
            }
            Err(idx) if idx > 0 => {
                // Not exact match - return nearest preceding position
                let (_, line, col) = self.offset_index[idx - 1];
                Some((line, col))
            }
            Err(_) => {
                // Before first position
                self.offset_index.first().map(|(_, line, col)| (*line, *col))
            }
        }
    }

    /// Linear search fallback for source-to-screen (used before index built)
    fn source_to_screen_linear(&self, offset: usize) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None; // (line, col, offset_diff)

        for (line_idx, line) in self.lines.iter().enumerate() {
            for (col_idx, mc) in line.iter().enumerate() {
                if let Some(src_off) = mc.source_offset {
                    if src_off == offset {
                        return Some((line_idx, col_idx));
                    }
                    if src_off <= offset {
                        let diff = offset - src_off;
                        let dominated = best.as_ref().is_some_and(|b| diff >= b.2);
                        if !dominated {
                            best = Some((line_idx, col_idx, diff));
                        }
                    }
                }
            }
        }

        best.map(|(l, c, _)| (l, c))
    }

    /// Get all screen positions that map to a source range
    /// Useful for highlighting selections
    pub fn source_range_to_screen(&self, start: usize, end: usize) -> Vec<(usize, usize)> {
        let mut positions = Vec::new();

        for (line_idx, line) in self.lines.iter().enumerate() {
            for (col_idx, mc) in line.iter().enumerate() {
                if let Some(offset) = mc.source_offset {
                    if offset >= start && offset < end {
                        positions.push((line_idx, col_idx));
                    }
                }
            }
        }

        positions
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor navigation helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Find the next valid cursor position after the given source offset
    /// O(log n) using binary search on sorted index
    #[inline]
    pub fn next_cursor_position(&self, current_offset: usize) -> Option<usize> {
        if !self.index_built {
            return None;
        }
        // partition_point finds first index where predicate is false
        // i.e., first offset > current_offset
        let idx = self.offset_index.partition_point(|(o, _, _)| *o <= current_offset);
        self.offset_index.get(idx).map(|(o, _, _)| *o)
    }

    /// Find the previous valid cursor position before the given source offset
    /// O(log n) using binary search on sorted index
    #[inline]
    pub fn prev_cursor_position(&self, current_offset: usize) -> Option<usize> {
        if !self.index_built {
            return None;
        }
        // partition_point finds first index where offset >= current
        // so idx - 1 is the last offset < current
        let idx = self.offset_index.partition_point(|(o, _, _)| *o < current_offset);
        idx.checked_sub(1).map(|i| self.offset_index[i].0)
    }

    /// Get the first valid source offset
    pub fn first_offset(&self) -> Option<usize> {
        self.offset_index.first().map(|(o, _, _)| *o)
    }

    /// Get the last valid source offset
    pub fn last_offset(&self) -> Option<usize> {
        self.offset_index.last().map(|(o, _, _)| *o)
    }
}

impl Default for LayoutMap {
    #[inline]
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            offset_index: Vec::new(),
            index_built: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_span() {
        let span = SourceSpan::new(10, 20);
        assert_eq!(span.len(), 10);
        assert!(span.contains(10));
        assert!(span.contains(19));
        assert!(!span.contains(20));
        assert!(!span.contains(9));
    }

    #[test]
    fn test_mapped_char() {
        let mc = MappedChar::with_source('a', 42);
        assert_eq!(mc.ch, 'a');
        assert_eq!(mc.source_offset, Some(42));
        assert!(!mc.is_synthetic());

        let synthetic = MappedChar::synthetic(' ');
        assert!(synthetic.is_synthetic());
    }

    #[test]
    fn test_layout_map_basic() {
        let mut map = LayoutMap::new(100);

        // Line 0: "  Hello" (2 synthetic spaces + "Hello" at offsets 0-4)
        map.push_line(vec![
            MappedChar::synthetic(' '),
            MappedChar::synthetic(' '),
            MappedChar::with_source('H', 0),
            MappedChar::with_source('e', 1),
            MappedChar::with_source('l', 2),
            MappedChar::with_source('l', 3),
            MappedChar::with_source('o', 4),
        ]);

        map.build_index();

        // Screen to source
        assert_eq!(map.screen_to_source(0, 0), None); // Synthetic
        assert_eq!(map.screen_to_source(0, 2), Some(0)); // 'H'
        assert_eq!(map.screen_to_source(0, 6), Some(4)); // 'o'

        // Source to screen
        assert_eq!(map.source_to_screen(0), Some((0, 2))); // offset 0 -> (0, 2)
        assert_eq!(map.source_to_screen(4), Some((0, 6))); // offset 4 -> (0, 6)
    }

    #[test]
    fn test_cursor_navigation() {
        let mut map = LayoutMap::new(10);

        map.push_line(vec![
            MappedChar::with_source('a', 0),
            MappedChar::with_source('b', 1),
            MappedChar::with_source('c', 2),
        ]);

        map.build_index();

        // first_offset is first char, last_offset is end position (after last char)
        assert_eq!(map.first_offset(), Some(0));
        assert_eq!(map.last_offset(), Some(3)); // End position is now in index

        // Forward navigation
        assert_eq!(map.next_cursor_position(0), Some(1));
        assert_eq!(map.next_cursor_position(1), Some(2));
        assert_eq!(map.next_cursor_position(2), Some(3)); // To end position
        assert_eq!(map.next_cursor_position(3), None);    // No more forward

        // Backward navigation
        assert_eq!(map.prev_cursor_position(3), Some(2));
        assert_eq!(map.prev_cursor_position(2), Some(1));
        assert_eq!(map.prev_cursor_position(1), Some(0));
        assert_eq!(map.prev_cursor_position(0), None);
    }

    #[test]
    fn test_end_of_line_cursor() {
        let mut map = LayoutMap::new(10);

        // Line with synthetic margin + content
        map.push_line(vec![
            MappedChar::synthetic(' '),
            MappedChar::synthetic(' '),
            MappedChar::with_source('H', 0),
            MappedChar::with_source('i', 1),
        ]);

        map.build_index();

        // Clicking past end of line should position at end of content
        assert_eq!(map.screen_to_source_nearest(0, 10), Some(2)); // After 'i'

        // Screen position for end-of-line offset
        assert_eq!(map.source_to_screen(2), Some((0, 4))); // One past 'i' column
    }
}
