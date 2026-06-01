//! Markdown parser for the read-only renderer.
//!
//! The AST keeps only the data needed for visual rendering. Source byte offsets
//! were removed with the editor path because read mode never maps screen
//! positions back to markdown bytes.

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use crate::chart::Chart;

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
    /// A ` ```chart ` block whose YAML parsed into a valid chart spec. Malformed
    /// chart blocks stay `CodeBlock` so they degrade to legible YAML.
    Chart {
        chart: Chart,
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
        /// Per-column horizontal alignment (defaults to Left when unspecified).
        aligns: Vec<CellAlign>,
    }
}

/// Per-column table alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellAlign {
    Left,
    Center,
    Right,
}

// ═══════════════════════════════════════════════════════════════════════════
// LIST ITEM
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct ListItem {
    /// Leading inline text of the item (rendered on the marker line).
    pub spans: Vec<Span>,
    /// Block-level content following the leading text: code blocks, blockquotes,
    /// nested lists, and additional paragraphs.
    pub blocks: Vec<Element>,
    /// GFM task-list state: `None` for a normal item, `Some(true/false)` for a
    /// checked/unchecked checkbox item.
    pub task: Option<bool>,
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
    Image { url: String, alt: String },
    Strikethrough(String),
    /// A footnote reference. `number` is resolved after parsing (order of first
    /// reference); `label` is the source identifier used to dedupe references.
    FootnoteRef { label: String, number: usize },
    SoftBreak,
    HardBreak,
}

const EMPHASIS: u8 = 1;
const STRONG: u8 = 2;
const STRIKETHROUGH: u8 = 4;

/// Map a raw inline-HTML tag (e.g. `<strong>`, `</em>`) to a style flag and
/// whether it opens (`true`) or closes (`false`) that style. Returns `None` for
/// tags we don't translate.
fn html_style_toggle(tag: &str) -> Option<(u8, bool)> {
    let t = tag.trim().strip_prefix('<')?.strip_suffix('>')?;
    let (close, body) = match t.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let name = body
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let flag = match name.as_str() {
        "strong" | "b" => STRONG,
        "em" | "i" => EMPHASIS,
        "del" | "s" | "strike" => STRIKETHROUGH,
        _ => return None,
    };
    Some((flag, !close))
}

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
    let mut footnote_defs: Vec<(String, Vec<Element>)> = Vec::new();
    let mut event_iter = parser.peekable();

    while let Some(event) = event_iter.next() {
        if let Event::Start(Tag::FootnoteDefinition(label)) = &event {
            let label = label.to_string();
            let inner = collect_footnote_def(&mut event_iter);
            footnote_defs.push((label, inner));
            continue;
        }
        if let Some(element) = parse_event(event, &mut event_iter) {
            elements.push(element);
        }
    }

    resolve_footnotes(&mut elements, footnote_defs);
    elements
}

/// Collect the block content of a footnote definition until its end tag.
fn collect_footnote_def<'a, I>(iter: &mut std::iter::Peekable<I>) -> Vec<Element>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut elements = Vec::new();
    loop {
        match iter.next() {
            Some(Event::End(TagEnd::FootnoteDefinition)) => break,
            Some(e) => {
                if let Some(el) = parse_event(e, iter) {
                    elements.push(el);
                }
            }
            None => break,
        }
    }
    elements
}

/// Number footnote references by order of first reference, then append a
/// footnotes section (a rule followed by numbered definitions) at the end.
fn resolve_footnotes(elements: &mut Vec<Element>, defs: Vec<(String, Vec<Element>)>) {
    use std::collections::HashMap;

    fn number_spans(spans: &mut [Span], map: &mut HashMap<String, usize>, next: &mut usize) {
        for s in spans {
            if let SpanKind::FootnoteRef { label, number } = &mut s.kind {
                let n = *map.entry(label.clone()).or_insert_with(|| {
                    let v = *next;
                    *next += 1;
                    v
                });
                *number = n;
            }
        }
    }
    fn walk(els: &mut [Element], map: &mut HashMap<String, usize>, next: &mut usize) {
        for el in els {
            match el {
                Element::Heading { spans, .. } | Element::Paragraph { spans } => {
                    number_spans(spans, map, next)
                }
                Element::BlockQuote { elements } => walk(elements, map, next),
                Element::List { items, .. } => {
                    for it in items {
                        number_spans(&mut it.spans, map, next);
                        walk(&mut it.blocks, map, next);
                    }
                }
                _ => {}
            }
        }
    }

    let mut map: HashMap<String, usize> = HashMap::new();
    let mut next = 1usize;
    walk(elements, &mut map, &mut next);

    if defs.is_empty() {
        return;
    }

    // Order definitions by their reference number; unreferenced ones trail.
    let mut numbered: Vec<(usize, Vec<Element>)> = defs
        .into_iter()
        .map(|(label, inner)| {
            let n = *map.entry(label.clone()).or_insert_with(|| {
                let v = next;
                next += 1;
                v
            });
            (n, inner)
        })
        .collect();
    numbered.sort_by_key(|(n, _)| *n);

    elements.push(Element::HorizontalRule {});
    for (n, inner) in numbered {
        // Prefix the first paragraph with "n. "; keep any further blocks as-is.
        let mut blocks = inner.into_iter();
        let first = blocks.next();
        let mut spans = vec![Span { kind: SpanKind::Text(format!("{n}. ")) }];
        let mut trailing: Vec<Element> = Vec::new();
        match first {
            Some(Element::Paragraph { spans: ps }) => spans.extend(ps),
            Some(other) => trailing.push(other),
            None => {}
        }
        elements.push(Element::Paragraph { spans });
        trailing.extend(blocks);
        elements.extend(trailing);
    }
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
        Event::Start(Tag::CodeBlock(kind)) => {
            let code = collect_text_until_end(iter, TagEnd::CodeBlock);
            // First word of the info string is the language / block type.
            let lang = match &kind {
                CodeBlockKind::Fenced(info) => {
                    info.split_whitespace().next().unwrap_or("").to_string()
                }
                CodeBlockKind::Indented => String::new(),
            };
            // Invalid chart falls through to a plain code block (fallback
            // contract: show the YAML rather than erroring).
            if lang == "chart"
                && let Some(chart) = Chart::parse(&code)
            {
                return Some(Element::Chart { chart });
            }
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
        Event::Start(Tag::Table(alignments)) => {
            let aligns = alignments
                .iter()
                .map(|a| match a {
                    Alignment::Right => CellAlign::Right,
                    Alignment::Center => CellAlign::Center,
                    _ => CellAlign::Left,
                })
                .collect();
            let (headers, rows) = collect_table(iter);
            Some(Element::Table {
                headers,
                rows,
                aligns,
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
            SpanKind::Image { alt, .. } => text.push_str(alt),
            SpanKind::FootnoteRef { number, .. } => text.push_str(&superscript(*number)),
            SpanKind::SoftBreak | SpanKind::HardBreak => text.push(' '),
        }
    }
    text
}

/// Render a number as Unicode superscript digits (e.g. 12 → "¹²").
pub fn superscript(n: usize) -> String {
    const S: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    n.to_string()
        .chars()
        .map(|c| S[(c as u8 - b'0') as usize])
        .collect()
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
            Some(Event::FootnoteReference(label)) => {
                spans.push(Span {
                    kind: SpanKind::FootnoteRef { label: label.into_string(), number: 0 },
                });
            }
            Some(Event::Start(Tag::Image { dest_url, .. })) => {
                let alt = collect_image_alt(iter);
                spans.push(Span {
                    kind: SpanKind::Image { url: dest_url.into_string(), alt },
                });
            }
            Some(Event::InlineHtml(t)) => {
                if let Some((flag, open)) = html_style_toggle(&t) {
                    if open { style_flags |= flag } else { style_flags &= !flag }
                }
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

/// Collect an image's alt text (the events between Image start and end).
fn collect_image_alt<'a, I>(iter: &mut I) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();
    for event in iter {
        match event {
            Event::End(TagEnd::Image) => break,
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
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
                let mut blocks: Vec<Element> = Vec::new();
                let mut style_flags = 0u8;
                let mut task = None;
                // The first paragraph becomes the item's leading text; any
                // further paragraph/block content is collected into `blocks`.
                let mut seen_leading = false;

                loop {
                    match iter.next() {
                        Some(Event::End(TagEnd::Item)) => break,
                        Some(Event::TaskListMarker(checked)) => task = Some(checked),
                        Some(Event::Text(t)) if blocks.is_empty() => {
                            let text = t.into_string();
                            spans.push(Span { kind: styled_text_kind(text, style_flags) });
                        }
                        Some(Event::Code(t)) if blocks.is_empty() => {
                            spans.push(Span { kind: SpanKind::Code(t.into_string()) });
                        }
                        Some(Event::FootnoteReference(label)) if blocks.is_empty() => {
                            spans.push(Span {
                                kind: SpanKind::FootnoteRef { label: label.into_string(), number: 0 },
                            });
                        }
                        Some(Event::Start(Tag::Image { dest_url, .. })) if blocks.is_empty() => {
                            let alt = collect_image_alt(iter);
                            spans.push(Span {
                                kind: SpanKind::Image { url: dest_url.into_string(), alt },
                            });
                        }
                        Some(Event::InlineHtml(t)) if blocks.is_empty() => {
                            if let Some((flag, open)) = html_style_toggle(&t) {
                                if open { style_flags |= flag } else { style_flags &= !flag }
                            }
                        }
                        Some(Event::Start(Tag::Emphasis)) => style_flags |= EMPHASIS,
                        Some(Event::End(TagEnd::Emphasis)) => style_flags &= !EMPHASIS,
                        Some(Event::Start(Tag::Strong)) => style_flags |= STRONG,
                        Some(Event::End(TagEnd::Strong)) => style_flags &= !STRONG,
                        Some(Event::Start(Tag::Strikethrough)) => style_flags |= STRIKETHROUGH,
                        Some(Event::End(TagEnd::Strikethrough)) => style_flags &= !STRIKETHROUGH,
                        Some(Event::Start(Tag::Paragraph)) => {
                            let para_spans = collect_spans_until_end(iter, TagEnd::Paragraph);
                            if !seen_leading && blocks.is_empty() {
                                spans.extend(para_spans);
                                seen_leading = true;
                            } else {
                                blocks.push(Element::Paragraph { spans: para_spans });
                            }
                        }
                        Some(Event::SoftBreak) if blocks.is_empty() => {
                            spans.push(Span { kind: SpanKind::SoftBreak });
                        }
                        // Any other block element (code block, blockquote,
                        // nested list, heading, rule, table) is parsed as a
                        // proper child block of this item.
                        Some(e) => {
                            if let Some(el) = parse_event(e, iter) {
                                blocks.push(el);
                            }
                        }
                        None => break,
                    }
                }

                items.push(ListItem { spans, blocks, task });
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
