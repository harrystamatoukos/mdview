//! Document model for centralized source and AST management
//!
//! This module provides a single source of truth for the markdown content,
//! combining raw source text with the parsed AST and dirty tracking.

use crate::parser::{parse, Element};

/// Central document model owning source text and parsed AST
///
/// The Document struct eliminates redundant state by combining:
/// - Raw markdown source
/// - Parsed element tree (AST)
/// - Dirty flags for parse state and save state
///
/// # Usage
///
/// ```ignore
/// let mut doc = Document::new(content);
/// doc.ensure_parsed(); // Parse if needed
/// let elements = doc.elements(); // Access AST
/// ```
#[derive(Debug, Clone)]
pub struct Document {
    /// Raw markdown source text
    source: String,
    /// Parsed elements (may be stale if parse_dirty is true)
    elements: Vec<Element>,
    /// True if source has changed since last parse
    parse_dirty: bool,
    /// True if source has changed since last save
    save_dirty: bool,
}

impl Document {
    /// Create a new document from source text
    ///
    /// The document starts with parse_dirty=true and will parse
    /// on first access to elements.
    pub fn new(source: String) -> Self {
        Self {
            source,
            elements: Vec::new(),
            parse_dirty: true,
            save_dirty: false,
        }
    }

    /// Create an empty document
    pub fn empty() -> Self {
        Self::new(String::new())
    }

    /// Get the raw source text
    #[inline]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Get the source length in bytes
    #[inline]
    pub fn source_len(&self) -> usize {
        self.source.len()
    }

    /// Get parsed elements, parsing if necessary
    ///
    /// This is the primary way to access the AST. If the source
    /// has been modified since the last parse, it will re-parse.
    pub fn elements(&mut self) -> &[Element] {
        self.ensure_parsed();
        &self.elements
    }

    /// Get elements without parsing (may be stale)
    ///
    /// Use this when you know the parse state is current
    /// and want to avoid the dirty check overhead.
    #[inline]
    pub fn elements_unchecked(&self) -> &[Element] {
        &self.elements
    }

    /// Check if source has been modified since last save
    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.save_dirty
    }

    /// Check if source needs re-parsing
    #[inline]
    pub fn needs_parse(&self) -> bool {
        self.parse_dirty
    }

    /// Mark the document as modified
    ///
    /// Call this after any source mutation. Sets both parse_dirty
    /// and save_dirty flags.
    pub fn mark_modified(&mut self) {
        self.parse_dirty = true;
        self.save_dirty = true;
    }

    /// Ensure the AST is up-to-date by parsing if needed
    ///
    /// Returns true if parsing was performed, false if already current.
    pub fn ensure_parsed(&mut self) -> bool {
        if self.parse_dirty {
            self.elements = parse(&self.source);
            self.parse_dirty = false;
            true
        } else {
            false
        }
    }

    /// Get mutable access to source for editing
    ///
    /// This marks the document as modified. After mutating,
    /// the AST will be re-parsed on next element access.
    pub fn source_mut(&mut self) -> &mut String {
        self.mark_modified();
        &mut self.source
    }

    /// Replace the entire source content
    pub fn set_source(&mut self, content: String) {
        self.source = content;
        self.mark_modified();
    }

    /// Mark document as saved (clears save_dirty flag)
    pub fn mark_saved(&mut self) {
        self.save_dirty = false;
    }

    /// Get word count (for status display)
    pub fn word_count(&self) -> usize {
        self.source.split_whitespace().count()
    }

    /// Get character count
    pub fn char_count(&self) -> usize {
        self.source.chars().count()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_document() {
        let doc = Document::new("# Hello".to_string());
        assert_eq!(doc.source(), "# Hello");
        assert!(doc.needs_parse());
        assert!(!doc.is_dirty());
    }

    #[test]
    fn test_ensure_parsed() {
        let mut doc = Document::new("# Hello".to_string());
        assert!(doc.needs_parse());

        // First parse
        let parsed = doc.ensure_parsed();
        assert!(parsed);
        assert!(!doc.needs_parse());
        assert!(!doc.elements_unchecked().is_empty());

        // Second call should not re-parse
        let parsed_again = doc.ensure_parsed();
        assert!(!parsed_again);
    }

    #[test]
    fn test_mark_modified() {
        let mut doc = Document::new("# Hello".to_string());
        doc.ensure_parsed();
        assert!(!doc.needs_parse());
        assert!(!doc.is_dirty());

        doc.mark_modified();
        assert!(doc.needs_parse());
        assert!(doc.is_dirty());
    }

    #[test]
    fn test_mark_saved() {
        let mut doc = Document::new("# Hello".to_string());
        doc.mark_modified();
        assert!(doc.is_dirty());

        doc.mark_saved();
        assert!(!doc.is_dirty());
        // parse_dirty should still be true
        assert!(doc.needs_parse());
    }

    #[test]
    fn test_source_mut() {
        let mut doc = Document::new("# Hello".to_string());
        doc.ensure_parsed();

        doc.source_mut().push_str(" World");
        assert_eq!(doc.source(), "# Hello World");
        assert!(doc.needs_parse());
        assert!(doc.is_dirty());
    }

    #[test]
    fn test_word_count() {
        let doc = Document::new("Hello world, how are you?".to_string());
        assert_eq!(doc.word_count(), 5);
    }

    #[test]
    fn test_elements_access() {
        let mut doc = Document::new("# Heading\n\nParagraph text.".to_string());

        // Access elements triggers parse
        let elements = doc.elements();
        assert_eq!(elements.len(), 2); // Heading + Paragraph
    }
}
