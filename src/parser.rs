//! Markdown parser for the read-only renderer.
//!
//! The AST keeps only the data needed for visual rendering. Source byte offsets
//! were removed with the editor path because read mode never maps screen
//! positions back to markdown bytes.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

// ═══════════════════════════════════════════════════════════════════════════
// ELEMENT - Parsed markdown block
// ═══════════════════════════════════════════════════════════════════════════

/// Parsed markdown element for rendering.
#[derive(Debug, Clone)]
pub enum Element {
    Heading {
        level: u8,
        spans: Vec<Span>,
        text: String,
    },
    Paragraph {
        spans: Vec<Span>,
    },
    CodeBlock {
        code: String,
    },
    BlockQuote {
        elements: Vec<Element>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    HorizontalRule {},
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LIST ITEM
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct ListItem {
    pub spans: Vec<Span>,
    pub nested: Option<Box<Element>>,
}

// ═══════════════════════════════════════════════════════════════════════════
// SPAN - Inline content
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct Span {
    pub kind: SpanKind,
}

#[derive(Debug, Clone)]
pub enum SpanKind {
    Text(String),
    Emphasis(String),
    Strong(String),
    StrongEmphasis(String),
    Code(String),
    Link { text: String, url: String },
    Strikethrough(String),
    SoftBreak,
    HardBreak,
}

const EMPHASIS: u8 = 1;
const STRONG: u8 = 2;
const STRIKETHROUGH: u8 = 4;

fn styled_text_kind(text: String, style_flags: u8) -> SpanKind {
    if style_flags & STRIKETHROUGH != 0 {
        SpanKind::Strikethrough(text)
    } else {
        match style_flags & (EMPHASIS | STRONG) {
            0 => SpanKind::Text(text),
            EMPHASIS => SpanKind::Emphasis(text),
            STRONG => SpanKind::Strong(text),
            _ => SpanKind::StrongEmphasis(text),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PARSER
// ═══════════════════════════════════════════════════════════════════════════

/// Parse markdown content into the read-only intermediate representation.
pub fn parse(content: &str) -> Vec<Element> {
    let options = Options::all();
    let parser = Parser::new_ext(content, options);

    let mut elements = Vec::new();
    let mut event_iter = parser.peekable();

    while let Some(event) = event_iter.next() {
        if let Some(element) = parse_event(event, &mut event_iter) {
            elements.push(element);
        }
    }

    elements
}

fn parse_event<'a, I>(
    event: Event<'a>,
    iter: &mut std::iter::Peekable<I>,
) -> Option<Element>
where
    I: Iterator<Item = Event<'a>>,
{
    match event {
        Event::Start(Tag::Heading { level, .. }) => {
            let spans = collect_spans_until_end(iter, TagEnd::Heading(level));
            let text = spans_to_text(&spans);
            Some(Element::Heading {
                level: heading_level_to_u8(level),
                spans,
                text,
            })
        }
        Event::Start(Tag::Paragraph) => {
            let spans = collect_spans_until_end(iter, TagEnd::Paragraph);
            Some(Element::Paragraph { spans })
        }
        Event::Start(Tag::CodeBlock(_)) => {
            let code = collect_text_until_end(iter, TagEnd::CodeBlock);
            Some(Element::CodeBlock { code })
        }
        Event::Start(Tag::BlockQuote(_)) => {
            let mut inner_elements = Vec::new();

            loop {
                match iter.next() {
                    Some(Event::End(TagEnd::BlockQuote(_))) => break,
                    Some(e) => {
                        if let Some(el) = parse_event(e, iter) {
                            inner_elements.push(el);
                        }
                    }
                    None => break,
                }
            }
            Some(Element::BlockQuote {
                elements: inner_elements,
            })
        }
        Event::Start(Tag::List(start_num)) => {
            let ordered = start_num.is_some();
            let items = collect_list_items(iter);
            Some(Element::List {
                ordered,
                start: start_num,
                items,
            })
        }
        Event::Rule => Some(Element::HorizontalRule {}),
        Event::Start(Tag::Table(_)) => {
            let (headers, rows) = collect_table(iter);
            Some(Element::Table {
                headers,
                rows,
            })
        }
        _ => None,
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Collect text until end tag.
fn collect_text_until_end<'a, I>(
    iter: &mut I,
    end_tag: TagEnd,
) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();

    for event in iter {
        match event {
            Event::End(ref tag) if *tag == end_tag => break,
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }

    text
}

fn spans_to_text(spans: &[Span]) -> String {
    let mut text = String::new();
    for span in spans {
        match &span.kind {
            SpanKind::Text(t)
            | SpanKind::Emphasis(t)
            | SpanKind::Strong(t)
            | SpanKind::StrongEmphasis(t)
            | SpanKind::Code(t)
            | SpanKind::Strikethrough(t) => text.push_str(t),
            SpanKind::Link { text: link_text, .. } => text.push_str(link_text),
            SpanKind::SoftBreak | SpanKind::HardBreak => text.push(' '),
        }
    }
    text
}

/// Collect spans until end tag.
fn collect_spans_until_end<'a, I>(
    iter: &mut std::iter::Peekable<I>,
    end_tag: TagEnd,
) -> Vec<Span>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut spans = Vec::new();
    let mut style_flags = 0u8;

    loop {
        match iter.next() {
            Some(Event::End(ref tag)) if *tag == end_tag => break,
            Some(Event::Text(t)) => {
                // Use into_string() to avoid allocation when CowStr is already owned
                let text = t.into_string();
                spans.push(Span { kind: styled_text_kind(text, style_flags) });
            }
            Some(Event::Code(t)) => {
                spans.push(Span { kind: SpanKind::Code(t.into_string()) });
            }
            Some(Event::Start(Tag::Emphasis)) => style_flags |= EMPHASIS,
            Some(Event::End(TagEnd::Emphasis)) => style_flags &= !EMPHASIS,
            Some(Event::Start(Tag::Strong)) => style_flags |= STRONG,
            Some(Event::End(TagEnd::Strong)) => style_flags &= !STRONG,
            Some(Event::Start(Tag::Strikethrough)) => style_flags |= STRIKETHROUGH,
            Some(Event::End(TagEnd::Strikethrough)) => style_flags &= !STRIKETHROUGH,
            Some(Event::Start(Tag::Link { dest_url, .. })) => {
                let text = collect_link_text(iter);
                spans.push(Span {
                    kind: SpanKind::Link {
                        text,
                        url: dest_url.into_string(),
                    },
                });
            }
            Some(Event::SoftBreak) => {
                spans.push(Span { kind: SpanKind::SoftBreak });
            }
            Some(Event::HardBreak) => {
                spans.push(Span { kind: SpanKind::HardBreak });
            }
            None => break,
            _ => {}
        }
    }

    spans
}

fn collect_link_text<'a, I>(iter: &mut I) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();

    for event in iter {
        match event {
            Event::End(TagEnd::Link) => break,
            Event::Text(t) => text.push_str(&t),
            _ => {}
        }
    }

    text
}

fn collect_list_items<'a, I>(
    iter: &mut std::iter::Peekable<I>,
) -> Vec<ListItem>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut items = Vec::new();

    loop {
        match iter.next() {
            Some(Event::Start(Tag::Item)) => {
                let mut spans = Vec::new();
                let mut nested = None;
                let mut style_flags = 0u8;

                loop {
                    match iter.next() {
                        Some(Event::End(TagEnd::Item)) => break,
                        Some(Event::Text(t)) => {
                            let text = t.into_string();
                            spans.push(Span { kind: styled_text_kind(text, style_flags) });
                        }
                        Some(Event::Code(t)) => {
                            spans.push(Span { kind: SpanKind::Code(t.into_string()) });
                        }
                        Some(Event::Start(Tag::Emphasis)) => style_flags |= EMPHASIS,
                        Some(Event::End(TagEnd::Emphasis)) => style_flags &= !EMPHASIS,
                        Some(Event::Start(Tag::Strong)) => style_flags |= STRONG,
                        Some(Event::End(TagEnd::Strong)) => style_flags &= !STRONG,
                        Some(Event::Start(Tag::Strikethrough)) => style_flags |= STRIKETHROUGH,
                        Some(Event::End(TagEnd::Strikethrough)) => style_flags &= !STRIKETHROUGH,
                        Some(Event::Start(Tag::Paragraph)) => {
                            let para_spans = collect_spans_until_end(iter, TagEnd::Paragraph);
                            spans.extend(para_spans);
                        }
                        Some(Event::Start(Tag::List(start))) => {
                            let sub_items = collect_list_items(iter);
                            nested = Some(Box::new(Element::List {
                                ordered: start.is_some(),
                                start,
                                items: sub_items,
                            }));
                        }
                        Some(Event::SoftBreak) => {
                            spans.push(Span { kind: SpanKind::SoftBreak });
                        }
                        None => break,
                        _ => {}
                    }
                }

                items.push(ListItem { spans, nested });
            }
            Some(Event::End(TagEnd::List(_))) => break,
            None => break,
            _ => {}
        }
    }

    items
}

fn collect_table<'a, I>(iter: &mut I) -> (Vec<String>, Vec<Vec<String>>)
where
    I: Iterator<Item = Event<'a>>,
{
    let mut headers = Vec::new();
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_cell = String::new();
    let mut in_header = false;

    for event in iter {
        match event {
            Event::Start(Tag::TableHead) => in_header = true,
            Event::End(TagEnd::TableHead) => {
                if !current_cell.is_empty() {
                    headers.push(std::mem::take(&mut current_cell));
                }
                in_header = false;
            }
            Event::Start(Tag::TableRow) => current_row.clear(),
            Event::End(TagEnd::TableRow) => {
                if !in_header && !current_row.is_empty() {
                    rows.push(std::mem::take(&mut current_row));
                }
            }
            Event::Start(Tag::TableCell) => current_cell.clear(),
            Event::End(TagEnd::TableCell) => {
                if in_header {
                    headers.push(std::mem::take(&mut current_cell));
                } else {
                    current_row.push(std::mem::take(&mut current_cell));
                }
            }
            Event::Text(t) => current_cell.push_str(&t),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }

    (headers, rows)
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY COMPATIBILITY - for code that doesn't need positions yet
// ═══════════════════════════════════════════════════════════════════════════

impl Element {
    /// Get the text content of a heading (legacy helper)
    #[allow(dead_code)] // exercised by tests / available API
    pub fn heading_text(&self) -> Option<&str> {
        match self {
            Element::Heading { text, .. } => Some(text),
            _ => None,
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
    fn test_empty_input() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn test_parse_heading() {
        let els = parse("# Hello");
        assert_eq!(els.len(), 1);
        match &els[0] {
            Element::Heading { level, text, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(text, "Hello");
            }
            other => panic!("expected heading, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_heading_levels() {
        for n in 1u8..=6 {
            let md = format!("{} H", "#".repeat(n as usize));
            match &parse(&md)[0] {
                Element::Heading { level, .. } => assert_eq!(*level, n),
                other => panic!("expected heading, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_heading_text_helper() {
        assert_eq!(parse("## Title")[0].heading_text(), Some("Title"));
        assert_eq!(parse("plain")[0].heading_text(), None);
    }

    #[test]
    fn test_parse_paragraph_inline() {
        match &parse("normal *em* **strong** `code`")[0] {
            Element::Paragraph { spans, .. } => {
                assert!(spans.iter().any(|s| matches!(&s.kind, SpanKind::Emphasis(t) if t == "em")));
                assert!(spans.iter().any(|s| matches!(&s.kind, SpanKind::Strong(t) if t == "strong")));
                assert!(spans.iter().any(|s| matches!(&s.kind, SpanKind::Code(t) if t == "code")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_link() {
        match &parse("[text](https://example.com)")[0] {
            Element::Paragraph { spans, .. } => {
                assert!(spans.iter().any(|s| matches!(&s.kind,
                    SpanKind::Link { text, url } if text == "text" && url == "https://example.com")));
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_code_block() {
        match &parse("```rust\nfn main() {}\n```")[0] {
            Element::CodeBlock { code, .. } => {
                assert!(code.contains("fn main()"));
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_unordered_list() {
        match &parse("- a\n- b\n- c")[0] {
            Element::List { ordered, items, .. } => {
                assert!(!ordered);
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_ordered_list() {
        match &parse("1. one\n2. two")[0] {
            Element::List { ordered, start, items, .. } => {
                assert!(ordered);
                assert_eq!(*start, Some(1));
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_blockquote() {
        assert!(matches!(&parse("> quoted")[0], Element::BlockQuote { .. }));
    }

    #[test]
    fn test_parse_horizontal_rule() {
        assert!(matches!(&parse("---")[0], Element::HorizontalRule { .. }));
    }

}
