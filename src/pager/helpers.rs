//! Helper functions for text detection and analysis
//!
//! Pure functions for detecting markdown structures like list items,
//! blockquotes, and headings. Separated for clarity and testability.

/// Info about a detected list item
pub struct ListItemInfo {
    /// The marker to use for continuation (e.g., "- " or "2. ")
    pub marker: String,
    /// Length of the marker in the current line (for detecting empty items)
    pub marker_len: usize,
    /// Content after the marker
    pub content: String,
}

/// Detect heading level from line content
/// Returns Some(level) if line starts with # prefix, None otherwise
pub fn detect_heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }

    // Count consecutive # characters
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();

    // Verify it's followed by space (valid markdown heading)
    if hash_count >= 1 && hash_count <= 6 {
        let rest = &trimmed[hash_count..];
        if rest.is_empty() || rest.starts_with(' ') {
            return Some(hash_count as u8);
        }
    }

    None
}

/// Detect if line is a list item
/// Returns info about the list item including marker and content
pub fn detect_list_item(line: &str) -> Option<ListItemInfo> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();

    // Unordered list: -, *, or + followed by space
    if let Some(rest) = trimmed.strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        // Safe: strip_prefix succeeded, so first char exists
        let marker_char = trimmed.chars().next().expect("marker char exists after strip_prefix");
        let marker = format!("{}{} ", " ".repeat(indent), marker_char);
        return Some(ListItemInfo {
            marker,
            marker_len: indent + 2, // marker char + space
            content: rest.to_string(),
        });
    }

    // Ordered list: number followed by . or ) and space
    let mut chars = trimmed.chars().peekable();
    let mut num = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_ascii_digit() {
            num.push(ch);
            chars.next();
        } else {
            break;
        }
    }

    if !num.is_empty() {
        if let Some(&delim) = chars.peek() {
            if delim == '.' || delim == ')' {
                chars.next();
                if chars.peek() == Some(&' ') {
                    chars.next();
                    let rest: String = chars.collect();
                    // For continuation, increment the number
                    let next_num: u64 = num.parse().unwrap_or(1) + 1;
                    let marker = format!("{}{}{} ", " ".repeat(indent), next_num, delim);
                    return Some(ListItemInfo {
                        marker,
                        marker_len: indent + num.len() + 2, // number + delimiter + space
                        content: rest,
                    });
                }
            }
        }
    }

    None
}

/// Detect if line is a blockquote
/// Returns the marker to use for continuation (e.g., "> ")
pub fn detect_blockquote(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("> ") {
        Some("> ".to_string())
    } else if trimmed.starts_with(">") {
        Some("> ".to_string())
    } else {
        None
    }
}

/// Get the character before a byte offset in O(1) instead of O(n)
/// Works by looking back at most 4 bytes (max UTF-8 char size)
pub fn char_before(content: &str, byte_pos: usize) -> Option<char> {
    if byte_pos == 0 || byte_pos > content.len() {
        return None;
    }
    let start = byte_pos.saturating_sub(4);
    content[start..byte_pos].chars().next_back()
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_heading_level() {
        assert_eq!(detect_heading_level("# Heading 1"), Some(1));
        assert_eq!(detect_heading_level("## Heading 2"), Some(2));
        assert_eq!(detect_heading_level("### Heading 3"), Some(3));
        assert_eq!(detect_heading_level("######"), Some(6));
        assert_eq!(detect_heading_level("####### Too many"), None); // 7 is invalid
        assert_eq!(detect_heading_level("Not a heading"), None);
        assert_eq!(detect_heading_level("#NoSpace"), None);
    }

    #[test]
    fn test_detect_list_item_unordered() {
        let item = detect_list_item("- Item").unwrap();
        assert_eq!(item.marker, "- ");
        assert_eq!(item.content, "Item");

        let item = detect_list_item("* Another").unwrap();
        assert_eq!(item.marker, "* ");
        assert_eq!(item.content, "Another");

        let item = detect_list_item("+ Plus").unwrap();
        assert_eq!(item.marker, "+ ");
        assert_eq!(item.content, "Plus");
    }

    #[test]
    fn test_detect_list_item_ordered() {
        let item = detect_list_item("1. First").unwrap();
        assert_eq!(item.marker, "2. "); // Incremented for continuation
        assert_eq!(item.content, "First");

        let item = detect_list_item("10) Tenth").unwrap();
        assert_eq!(item.marker, "11) ");
        assert_eq!(item.content, "Tenth");
    }

    #[test]
    fn test_detect_list_item_indented() {
        let item = detect_list_item("  - Indented").unwrap();
        assert_eq!(item.marker, "  - ");
        assert_eq!(item.marker_len, 4);
    }

    #[test]
    fn test_detect_list_item_none() {
        assert!(detect_list_item("Regular text").is_none());
        assert!(detect_list_item("-No space").is_none());
        assert!(detect_list_item("1No delimiter").is_none());
    }

    #[test]
    fn test_detect_blockquote() {
        assert_eq!(detect_blockquote("> Quote"), Some("> ".to_string()));
        assert_eq!(detect_blockquote(">Tight quote"), Some("> ".to_string()));
        assert!(detect_blockquote("Not a quote").is_none());
    }

    #[test]
    fn test_char_before() {
        let content = "Hello, World!";
        assert_eq!(char_before(content, 0), None);
        assert_eq!(char_before(content, 1), Some('H'));
        assert_eq!(char_before(content, 5), Some('o'));
        assert_eq!(char_before(content, 7), Some(' '));
    }

    #[test]
    fn test_char_before_utf8() {
        let content = "Héllo";
        assert_eq!(char_before(content, 3), Some('é')); // é is 2 bytes
    }
}
