//! Position mapping for WYSIWYG editing
//!
//! This module provides bidirectional mapping between:
//! - Source positions (byte offsets in the markdown file)
//! - Screen positions (line, column in the rendered view)
//!
//! The key insight: every visible character in the rendered view
//! either maps to a source byte offset, or is "synthetic" (margins,
//! borders, list bullets) with no source position.

// Re-export types from primitives (single source of truth)
pub use crate::primitives::{ByteOffset, ScreenPos, SourceSpan};

// ═══════════════════════════════════════════════════════════════════════════
// FORMATTING SPAN - Tracks inline formatting markers in source
// ═══════════════════════════════════════════════════════════════════════════

/// Type of inline formatting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormattingKind {
    /// **bold** - 2 asterisks each side
    Strong,
    /// *italic* - 1 asterisk each side
    Emphasis,
    /// ***bold italic*** - 3 asterisks each side
    StrongEmphasis,
    /// `code` - 1 backtick each side
    Code,
    /// ~~strikethrough~~ - 2 tildes each side
    Strikethrough,
}

impl FormattingKind {
    /// Number of characters in the opening/closing marker
    pub fn marker_len(&self) -> usize {
        match self {
            FormattingKind::Strong => 2,        // **
            FormattingKind::Emphasis => 1,      // *
            FormattingKind::StrongEmphasis => 3, // ***
            FormattingKind::Code => 1,          // `
            FormattingKind::Strikethrough => 2, // ~~
        }
    }

    /// The marker string (for insertion)
    pub fn marker_str(&self) -> &'static str {
        match self {
            FormattingKind::Strong => "**",
            FormattingKind::Emphasis => "*",
            FormattingKind::StrongEmphasis => "***",
            FormattingKind::Code => "`",
            FormattingKind::Strikethrough => "~~",
        }
    }
}

/// A span of formatted text with its marker positions
///
/// For source: `Hello **world** there`
/// - marker_start: byte offset of first `*`
/// - content_start: byte offset of `w` in `world`
/// - content_end: byte offset after `d` in `world`
/// - marker_end: byte offset after second `*`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormattingSpan {
    /// Type of formatting (bold, italic, etc.)
    pub kind: FormattingKind,
    /// Byte offset where opening marker begins (first char of `**`)
    pub marker_start: usize,
    /// Byte offset where content begins (after opening marker)
    pub content_start: usize,
    /// Byte offset where content ends (before closing marker)
    pub content_end: usize,
    /// Byte offset after closing marker
    pub marker_end: usize,
}

impl FormattingSpan {
    /// Create a new formatting span
    pub fn new(
        kind: FormattingKind,
        marker_start: usize,
        content_start: usize,
        content_end: usize,
        marker_end: usize,
    ) -> Self {
        Self {
            kind,
            marker_start,
            content_start,
            content_end,
            marker_end,
        }
    }

    /// Create from content range (calculates marker positions)
    pub fn from_content_range(kind: FormattingKind, content_start: usize, content_end: usize) -> Self {
        let marker_len = kind.marker_len();
        Self {
            kind,
            marker_start: content_start.saturating_sub(marker_len),
            content_start,
            content_end,
            marker_end: content_end + marker_len,
        }
    }

    /// Check if a source offset is at the start boundary of content
    pub fn is_at_content_start(&self, offset: usize) -> bool {
        offset == self.content_start
    }

    /// Check if a source offset is at the end boundary of content
    pub fn is_at_content_end(&self, offset: usize) -> bool {
        offset == self.content_end
    }

    /// Check if a source offset is inside the content (not markers)
    pub fn contains_content(&self, offset: usize) -> bool {
        offset >= self.content_start && offset < self.content_end
    }

    /// Check if a source offset is inside the opening marker
    pub fn is_in_opening_marker(&self, offset: usize) -> bool {
        offset >= self.marker_start && offset < self.content_start
    }

    /// Check if a source offset is inside the closing marker
    pub fn is_in_closing_marker(&self, offset: usize) -> bool {
        offset >= self.content_end && offset < self.marker_end
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
    pub source_offset: Option<ByteOffset>,
}

impl MappedChar {
    /// Create a mapped character with a source position
    pub fn with_source(ch: char, offset: usize) -> Self {
        Self {
            ch,
            source_offset: Some(ByteOffset(offset)),
        }
    }

    /// Create a mapped character with a ByteOffset source position
    pub fn with_byte_offset(ch: char, offset: ByteOffset) -> Self {
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
    offset_index: Vec<(ByteOffset, usize, usize)>,
    /// Formatting spans (bold, italic, code, etc.) for boundary detection
    formatting_spans: Vec<FormattingSpan>,
    /// Whether the index has been built
    index_built: bool,
    /// Left margin where content starts (for cursor on empty lines)
    content_margin: usize,
}

impl LayoutMap {
    /// Create a new empty layout map
    #[inline]
    pub fn new(_source_len: usize) -> Self {
        Self {
            lines: Vec::new(),
            offset_index: Vec::new(),
            formatting_spans: Vec::new(),
            index_built: false,
            content_margin: 0,
        }
    }

    /// Set the content margin (left indentation for optimal reading)
    pub fn set_content_margin(&mut self, margin: usize) {
        self.content_margin = margin;
    }

    /// Get the content margin
    pub fn content_margin(&self) -> usize {
        self.content_margin
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
            let mut last_on_line: Option<(ByteOffset, usize, char)> = None;

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
    pub fn screen_to_source(&self, line: usize, col: usize) -> Option<ByteOffset> {
        self.lines
            .get(line)
            .and_then(|l| l.get(col))
            .and_then(|mc| mc.source_offset)
    }

    /// Get source offset for screen position, finding nearest if exact not available
    /// Handles empty lines by finding the nearest content line
    #[inline]
    pub fn screen_to_source_nearest(&self, line: usize, col: usize) -> Option<ByteOffset> {
        // Try to find offset on the current line first
        if let Some(offset) = self.find_offset_on_line(line, col) {
            return Some(offset);
        }

        // Empty line - search nearby lines (prefer above for paragraph spacing intuition)
        self.find_nearest_offset_from_line(line)
    }

    /// Find source offset on a specific line, searching outward from column
    fn find_offset_on_line(&self, line: usize, col: usize) -> Option<ByteOffset> {
        let line_chars = self.lines.get(line)?;

        // Exact position
        if let Some(offset) = line_chars.get(col).and_then(|mc| mc.source_offset) {
            return Some(offset);
        }

        // Past end of line - return position after last char
        if col >= line_chars.len() {
            return line_chars.iter().rev()
                .find_map(|mc| mc.source_offset.map(|o| o + mc.ch.len_utf8()));
        }

        // Search left then right from position
        let left = line_chars[..col].iter().rev().find_map(|mc| mc.source_offset);
        let right = line_chars.get(col + 1..).and_then(|slice|
            slice.iter().find_map(|mc| mc.source_offset)
        );

        left.or(right)
    }

    /// Find nearest offset by searching lines above then below
    fn find_nearest_offset_from_line(&self, from_line: usize) -> Option<ByteOffset> {
        // Search lines above (return end of last content)
        for search_line in (0..from_line).rev() {
            if let Some(offset) = self.lines.get(search_line).and_then(|chars|
                chars.iter().rev().find_map(|mc| mc.source_offset.map(|o| o + mc.ch.len_utf8()))
            ) {
                return Some(offset);
            }
        }

        // Search lines below (return start of first content)
        for search_line in (from_line + 1)..self.lines.len() {
            if let Some(offset) = self.lines.get(search_line).and_then(|chars|
                chars.iter().find_map(|mc| mc.source_offset)
            ) {
                return Some(offset);
            }
        }

        None
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Source → Screen mapping
    // ─────────────────────────────────────────────────────────────────────────

    /// Get screen position for a source offset
    /// Uses binary search for O(log n) lookup. Respects content margin for empty lines.
    #[inline]
    pub fn source_to_screen(&self, offset: ByteOffset) -> Option<ScreenPos> {
        if !self.index_built || self.offset_index.is_empty() {
            return self.source_to_screen_linear(offset);
        }

        match self.offset_index.binary_search_by_key(&offset, |(o, _, _)| *o) {
            // Exact match - return the position directly
            Ok(idx) => {
                let (_, line, col) = self.offset_index[idx];
                Some(ScreenPos::new(line, col))
            }

            // No exact match - cursor is between mapped positions
            Err(idx) => {
                // Get the previous mapped position
                let (prev_offset, prev_line, prev_col) = if idx > 0 {
                    self.offset_index[idx - 1]
                } else {
                    return Some(ScreenPos::new(0, self.content_margin));
                };

                // Check distance from previous position
                let distance = offset - prev_offset; // ByteOffset - ByteOffset = usize

                // If cursor is close to previous position (1-3 chars, like after typing space/punctuation),
                // advance cursor on the same line
                if distance <= 3 {
                    let new_col = prev_col + distance;
                    return Some(ScreenPos::new(prev_line, new_col));
                }

                // Cursor is far from previous content - likely in paragraph break (after Enter)
                // Position on immediate next line
                let target_line = if prev_line + 1 < self.lines.len() {
                    prev_line + 1
                } else {
                    prev_line
                };

                Some(ScreenPos::new(target_line, self.content_margin))
            }
        }
    }

    /// Linear search fallback for source-to-screen (used before index built)
    fn source_to_screen_linear(&self, offset: ByteOffset) -> Option<ScreenPos> {
        let mut best: Option<(usize, usize, usize)> = None; // (line, col, offset_diff)

        for (line_idx, line) in self.lines.iter().enumerate() {
            for (col_idx, mc) in line.iter().enumerate() {
                let Some(src_off) = mc.source_offset else { continue };

                // Exact match
                if src_off == offset {
                    return Some(ScreenPos::new(line_idx, col_idx));
                }

                // Track closest position at or before target offset
                if src_off <= offset {
                    let diff = offset - src_off; // ByteOffset - ByteOffset = usize
                    let dominated = best.map_or(false, |(_, _, best_diff)| diff >= best_diff);
                    if !dominated {
                        best = Some((line_idx, col_idx, diff));
                    }
                }
            }
        }

        best.map(|(line, col, _)| ScreenPos::new(line, col))
    }

    /// Get all screen positions that map to a source range
    /// Useful for highlighting selections
    /// O(log n + k) where k is the number of positions in range
    pub fn source_range_to_screen(&self, start: ByteOffset, end: ByteOffset) -> Vec<ScreenPos> {
        if !self.index_built || self.offset_index.is_empty() || start >= end {
            return Vec::new();
        }

        // Binary search for first offset >= start
        let start_idx = self.offset_index.partition_point(|(o, _, _)| *o < start);

        // Binary search for first offset >= end
        let end_idx = self.offset_index.partition_point(|(o, _, _)| *o < end);

        // Collect positions in range - O(k) where k = end_idx - start_idx
        self.offset_index[start_idx..end_idx]
            .iter()
            .map(|(_, line, col)| ScreenPos::new(*line, *col))
            .collect()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor navigation helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Find the next valid cursor position after the given source offset
    /// O(log n) using binary search on sorted index
    #[inline]
    pub fn next_cursor_position(&self, current_offset: ByteOffset) -> Option<ByteOffset> {
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
    pub fn prev_cursor_position(&self, current_offset: ByteOffset) -> Option<ByteOffset> {
        if !self.index_built {
            return None;
        }
        // partition_point finds first index where offset >= current
        // so idx - 1 is the last offset < current
        let idx = self.offset_index.partition_point(|(o, _, _)| *o < current_offset);
        idx.checked_sub(1).map(|i| self.offset_index[i].0)
    }

    /// Get the first valid source offset
    pub fn first_offset(&self) -> Option<ByteOffset> {
        self.offset_index.first().map(|(o, _, _)| *o)
    }

    /// Get the last valid source offset
    pub fn last_offset(&self) -> Option<ByteOffset> {
        self.offset_index.last().map(|(o, _, _)| *o)
    }

    /// Check if a line has any mapped source content (not just synthetic chars)
    pub fn line_has_content(&self, line: usize) -> bool {
        self.lines.get(line).is_some_and(|chars| {
            chars.iter().any(|mc| mc.source_offset.is_some())
        })
    }

    /// Find the source offset where we should insert content to create a paragraph
    /// before the given visual line. Returns the end of the last content block above.
    pub fn find_insertion_point_before_line(&self, target_line: usize) -> Option<ByteOffset> {
        // Search backwards from target_line to find the last line with content
        for line in (0..target_line).rev() {
            if let Some(chars) = self.lines.get(line) {
                // Find the last mapped character on this line
                if let Some(mc) = chars.iter().rev().find(|mc| mc.source_offset.is_some()) {
                    // Return position AFTER this character
                    return mc.source_offset.map(|o| o + mc.ch.len_utf8());
                }
            }
        }
        // No content found above - insert at start
        Some(ByteOffset::ZERO)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Formatting span management
    // ─────────────────────────────────────────────────────────────────────────

    /// Add a formatting span (bold, italic, code, etc.)
    pub fn push_formatting_span(&mut self, span: FormattingSpan) {
        self.formatting_spans.push(span);
    }

    /// Clear all formatting spans (called before re-rendering)
    pub fn clear_formatting_spans(&mut self) {
        self.formatting_spans.clear();
    }

    /// Get all formatting spans
    pub fn formatting_spans(&self) -> &[FormattingSpan] {
        &self.formatting_spans
    }

    /// Find formatting span whose content boundary is at the given offset
    /// Returns Some if cursor is exactly at content_start or content_end
    pub fn formatting_at_boundary(&self, offset: usize) -> Option<&FormattingSpan> {
        self.formatting_spans.iter().find(|span| {
            span.is_at_content_start(offset) || span.is_at_content_end(offset)
        })
    }

    /// Find formatting span if cursor is at the START of its content
    pub fn formatting_at_content_start(&self, offset: usize) -> Option<&FormattingSpan> {
        self.formatting_spans.iter().find(|span| span.is_at_content_start(offset))
    }

    /// Find formatting span if cursor is at the END of its content
    pub fn formatting_at_content_end(&self, offset: usize) -> Option<&FormattingSpan> {
        self.formatting_spans.iter().find(|span| span.is_at_content_end(offset))
    }

    /// Find formatting span that contains the given offset in its content
    pub fn formatting_containing(&self, offset: usize) -> Option<&FormattingSpan> {
        self.formatting_spans.iter().find(|span| span.contains_content(offset))
    }

    /// Check if cursor is inside any formatted content
    pub fn is_in_formatted_content(&self, offset: usize) -> bool {
        self.formatting_spans.iter().any(|span| span.contains_content(offset))
    }

    /// Get all formatting spans of a specific kind
    pub fn formatting_spans_of_kind(&self, kind: FormattingKind) -> impl Iterator<Item = &FormattingSpan> {
        self.formatting_spans.iter().filter(move |span| span.kind == kind)
    }
}

impl Default for LayoutMap {
    #[inline]
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            offset_index: Vec::new(),
            formatting_spans: Vec::new(),
            index_built: false,
            content_margin: 0,
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
        use crate::primitives::ByteOffset;
        let span = SourceSpan::from_usize(10, 20);
        assert_eq!(span.len(), 10);
        assert!(span.contains(ByteOffset(10)));
        assert!(span.contains(ByteOffset(19)));
        assert!(!span.contains(ByteOffset(20)));
        assert!(!span.contains(ByteOffset(9)));
    }

    #[test]
    fn test_mapped_char() {
        let mc = MappedChar::with_source('a', 42);
        assert_eq!(mc.ch, 'a');
        assert_eq!(mc.source_offset, Some(ByteOffset(42)));
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
        assert_eq!(map.screen_to_source(0, 2), Some(ByteOffset(0))); // 'H'
        assert_eq!(map.screen_to_source(0, 6), Some(ByteOffset(4))); // 'o'

        // Source to screen
        assert_eq!(map.source_to_screen(ByteOffset(0)), Some(ScreenPos::new(0, 2))); // offset 0 -> (0, 2)
        assert_eq!(map.source_to_screen(ByteOffset(4)), Some(ScreenPos::new(0, 6))); // offset 4 -> (0, 6)
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
        assert_eq!(map.first_offset(), Some(ByteOffset(0)));
        assert_eq!(map.last_offset(), Some(ByteOffset(3))); // End position is now in index

        // Forward navigation
        assert_eq!(map.next_cursor_position(ByteOffset(0)), Some(ByteOffset(1)));
        assert_eq!(map.next_cursor_position(ByteOffset(1)), Some(ByteOffset(2)));
        assert_eq!(map.next_cursor_position(ByteOffset(2)), Some(ByteOffset(3))); // To end position
        assert_eq!(map.next_cursor_position(ByteOffset(3)), None);    // No more forward

        // Backward navigation
        assert_eq!(map.prev_cursor_position(ByteOffset(3)), Some(ByteOffset(2)));
        assert_eq!(map.prev_cursor_position(ByteOffset(2)), Some(ByteOffset(1)));
        assert_eq!(map.prev_cursor_position(ByteOffset(1)), Some(ByteOffset(0)));
        assert_eq!(map.prev_cursor_position(ByteOffset(0)), None);
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
        assert_eq!(map.screen_to_source_nearest(0, 10), Some(ByteOffset(2))); // After 'i'

        // Screen position for end-of-line offset
        assert_eq!(map.source_to_screen(ByteOffset(2)), Some(ScreenPos::new(0, 4))); // One past 'i' column
    }

    #[test]
    fn test_formatting_span_from_content_range() {
        // For "**bold**" - pulldown-cmark gives us "bold" at bytes 2-6
        // content_start=2, content_end=6
        let span = FormattingSpan::from_content_range(FormattingKind::Strong, 2, 6);

        assert_eq!(span.marker_start, 0);     // ** starts at byte 0
        assert_eq!(span.content_start, 2);    // content starts after **
        assert_eq!(span.content_end, 6);      // content ends before closing **
        assert_eq!(span.marker_end, 8);       // ** ends at byte 8

        // Boundary detection
        assert!(span.is_at_content_start(2));
        assert!(!span.is_at_content_start(3));
        assert!(span.is_at_content_end(6));
        assert!(!span.is_at_content_end(5));
    }

    #[test]
    fn test_formatting_span_italic() {
        // For "*italic*" - pulldown-cmark gives us "italic" at bytes 1-7
        let span = FormattingSpan::from_content_range(FormattingKind::Emphasis, 1, 7);

        assert_eq!(span.marker_start, 0);     // * starts at byte 0
        assert_eq!(span.content_start, 1);    // content starts after *
        assert_eq!(span.content_end, 7);      // content ends before closing *
        assert_eq!(span.marker_end, 8);       // * ends at byte 8
    }

    #[test]
    fn test_formatting_span_code() {
        // For "`code`" - pulldown-cmark gives us "code" at bytes 1-5
        let span = FormattingSpan::from_content_range(FormattingKind::Code, 1, 5);

        assert_eq!(span.marker_start, 0);     // ` starts at byte 0
        assert_eq!(span.content_start, 1);    // content starts after `
        assert_eq!(span.content_end, 5);      // content ends before closing `
        assert_eq!(span.marker_end, 6);       // ` ends at byte 6
    }

    #[test]
    fn test_formatting_span_strong_emphasis() {
        // For "***bold italic***" - pulldown-cmark gives us "bold italic" at bytes 3-14
        let span = FormattingSpan::from_content_range(FormattingKind::StrongEmphasis, 3, 14);

        assert_eq!(span.marker_start, 0);     // *** starts at byte 0
        assert_eq!(span.content_start, 3);    // content starts after ***
        assert_eq!(span.content_end, 14);     // content ends before closing ***
        assert_eq!(span.marker_end, 17);      // *** ends at byte 17
    }

    #[test]
    fn test_formatting_span_strikethrough() {
        // For "~~strikethrough~~" - pulldown-cmark gives us "strikethrough" at bytes 2-15
        let span = FormattingSpan::from_content_range(FormattingKind::Strikethrough, 2, 15);

        assert_eq!(span.marker_start, 0);     // ~~ starts at byte 0
        assert_eq!(span.content_start, 2);    // content starts after ~~
        assert_eq!(span.content_end, 15);     // content ends before closing ~~
        assert_eq!(span.marker_end, 17);      // ~~ ends at byte 17
    }

    #[test]
    fn test_layout_map_formatting_boundary() {
        let mut map = LayoutMap::new(100);

        // Simulate rendering "Hello **world** there"
        // "Hello " at 0-6, "world" at 8-13 (bold content), " there" at 15-21
        // The ** markers are at 6-8 and 13-15
        map.push_line(vec![
            MappedChar::with_source('H', 0),
            MappedChar::with_source('e', 1),
            MappedChar::with_source('l', 2),
            MappedChar::with_source('l', 3),
            MappedChar::with_source('o', 4),
            MappedChar::with_source(' ', 5),
            // Bold content starts here, markers are hidden
            MappedChar::with_source('w', 8),
            MappedChar::with_source('o', 9),
            MappedChar::with_source('r', 10),
            MappedChar::with_source('l', 11),
            MappedChar::with_source('d', 12),
            // Bold content ends, markers hidden
            MappedChar::with_source(' ', 15),
            MappedChar::with_source('t', 16),
            MappedChar::with_source('h', 17),
            MappedChar::with_source('e', 18),
            MappedChar::with_source('r', 19),
            MappedChar::with_source('e', 20),
        ]);

        // Add the formatting span for the bold text
        map.push_formatting_span(FormattingSpan::from_content_range(
            FormattingKind::Strong,
            8,  // content_start (where 'w' is)
            13, // content_end (after 'd')
        ));

        map.build_index();

        // Test boundary detection
        let span_at_start = map.formatting_at_content_start(8);
        assert!(span_at_start.is_some());
        assert_eq!(span_at_start.unwrap().kind, FormattingKind::Strong);

        let span_at_end = map.formatting_at_content_end(13);
        assert!(span_at_end.is_some());
        assert_eq!(span_at_end.unwrap().kind, FormattingKind::Strong);

        // Not at boundary
        assert!(map.formatting_at_content_start(9).is_none());
        assert!(map.formatting_at_content_end(12).is_none());
    }
}
