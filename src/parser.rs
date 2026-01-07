//! Markdown parser with source position tracking
//!
//! This module parses markdown into an AST while preserving byte offsets
//! for WYSIWYG editing support.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, HeadingLevel, CodeBlockKind};
use std::ops::Range;

use crate::primitives::SourceSpan;

// ═══════════════════════════════════════════════════════════════════════════
// ELEMENT - Parsed markdown block with source position
// ═══════════════════════════════════════════════════════════════════════════

/// Parsed markdown element for rendering, with source position
#[derive(Debug, Clone)]
pub enum Element {
    Heading {
        level: u8,
        text: String,
        source: SourceSpan,
    },
    Paragraph {
        spans: Vec<Span>,
        source: SourceSpan,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
        source: SourceSpan,
    },
    BlockQuote {
        elements: Vec<Element>,
        source: SourceSpan,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<ListItem>,
        source: SourceSpan,
    },
    HorizontalRule {
        source: SourceSpan,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        source: SourceSpan,
    },
}

impl Element {
    /// Get the source span for this element
    pub fn source(&self) -> SourceSpan {
        match self {
            Element::Heading { source, .. } => *source,
            Element::Paragraph { source, .. } => *source,
            Element::CodeBlock { source, .. } => *source,
            Element::BlockQuote { source, .. } => *source,
            Element::List { source, .. } => *source,
            Element::HorizontalRule { source } => *source,
            Element::Table { source, .. } => *source,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LIST ITEM
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct ListItem {
    pub spans: Vec<Span>,
    pub nested: Option<Box<Element>>,
    pub source: SourceSpan,
}

// ═══════════════════════════════════════════════════════════════════════════
// SPAN - Inline content with source position
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct Span {
    pub kind: SpanKind,
    pub source: SourceSpan,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Variants for future inline styling support
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

// ═══════════════════════════════════════════════════════════════════════════
// PARSER
// ═══════════════════════════════════════════════════════════════════════════

/// Event with its source range
type RangedEvent<'a> = (Event<'a>, Range<usize>);

/// Parse markdown content into our intermediate representation WITH source positions
pub fn parse(content: &str) -> Vec<Element> {
    let options = Options::all();
    let parser = Parser::new_ext(content, options);

    // Use into_offset_iter to get byte offsets with each event
    let offset_iter = parser.into_offset_iter();

    let mut elements = Vec::new();
    let mut event_iter = offset_iter.peekable();

    while let Some((event, range)) = event_iter.next() {
        if let Some(element) = parse_event(event, range, &mut event_iter) {
            elements.push(element);
        }
    }

    elements
}

fn parse_event<'a, I>(
    event: Event<'a>,
    range: Range<usize>,
    iter: &mut std::iter::Peekable<I>,
) -> Option<Element>
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    match event {
        Event::Start(Tag::Heading { level, .. }) => {
            let start = range.start;
            let (text, end) = collect_text_until_end_ranged(iter, TagEnd::Heading(level));
            Some(Element::Heading {
                level: heading_level_to_u8(level),
                text,
                source: SourceSpan::from_usize(start, end),
            })
        }
        Event::Start(Tag::Paragraph) => {
            let start = range.start;
            let (spans, end) = collect_spans_until_end_ranged(iter, TagEnd::Paragraph);
            Some(Element::Paragraph {
                spans,
                source: SourceSpan::from_usize(start, end),
            })
        }
        Event::Start(Tag::CodeBlock(kind)) => {
            let start = range.start;
            let language = match kind {
                CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.into_string()),
                _ => None,
            };
            let (code, end) = collect_text_until_end_ranged(iter, TagEnd::CodeBlock);
            Some(Element::CodeBlock {
                language,
                code,
                source: SourceSpan::from_usize(start, end),
            })
        }
        Event::Start(Tag::BlockQuote(_)) => {
            let start = range.start;
            let mut inner_elements = Vec::new();
            let mut end = range.end;

            loop {
                match iter.next() {
                    Some((Event::End(TagEnd::BlockQuote(_)), r)) => {
                        end = r.end;
                        break;
                    }
                    Some((e, r)) => {
                        if let Some(el) = parse_event(e, r, iter) {
                            inner_elements.push(el);
                        }
                    }
                    None => break,
                }
            }
            Some(Element::BlockQuote {
                elements: inner_elements,
                source: SourceSpan::from_usize(start, end),
            })
        }
        Event::Start(Tag::List(start_num)) => {
            let start = range.start;
            let ordered = start_num.is_some();
            let (items, end) = collect_list_items_ranged(iter);
            Some(Element::List {
                ordered,
                start: start_num,
                items,
                source: SourceSpan::from_usize(start, end),
            })
        }
        Event::Rule => Some(Element::HorizontalRule {
            source: SourceSpan::from_range(range),
        }),
        Event::Start(Tag::Table(_)) => {
            let start = range.start;
            let (headers, rows, end) = collect_table_ranged(iter);
            Some(Element::Table {
                headers,
                rows,
                source: SourceSpan::from_usize(start, end),
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

/// Collect text until end tag, returning text and end position
fn collect_text_until_end_ranged<'a, I>(
    iter: &mut I,
    end_tag: TagEnd,
) -> (String, usize)
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    let mut text = String::new();
    let mut end_pos = 0;

    for (event, range) in iter {
        end_pos = range.end;
        match event {
            Event::End(ref tag) if *tag == end_tag => break,
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }

    (text, end_pos)
}

/// Collect spans until end tag, returning spans and end position
fn collect_spans_until_end_ranged<'a, I>(
    iter: &mut std::iter::Peekable<I>,
    end_tag: TagEnd,
) -> (Vec<Span>, usize)
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    let mut spans = Vec::new();
    let mut emphasis_level = 0u8;
    let mut end_pos = 0;

    loop {
        match iter.next() {
            Some((Event::End(ref tag), range)) if *tag == end_tag => {
                end_pos = range.end;
                break;
            }
            Some((Event::Text(t), range)) => {
                // Use into_string() to avoid allocation when CowStr is already owned
                let text = t.into_string();
                let kind = match emphasis_level {
                    0 => SpanKind::Text(text),
                    1 => SpanKind::Emphasis(text),
                    2 => SpanKind::Strong(text),
                    _ => SpanKind::StrongEmphasis(text),
                };
                spans.push(Span {
                    kind,
                    source: SourceSpan::from_range(range),
                });
            }
            Some((Event::Code(t), range)) => {
                spans.push(Span {
                    kind: SpanKind::Code(t.into_string()),
                    source: SourceSpan::from_range(range),
                });
            }
            Some((Event::Start(Tag::Emphasis), _)) => emphasis_level |= 1,
            Some((Event::End(TagEnd::Emphasis), _)) => emphasis_level &= !1,
            Some((Event::Start(Tag::Strong), _)) => emphasis_level |= 2,
            Some((Event::End(TagEnd::Strong), _)) => emphasis_level &= !2,
            Some((Event::Start(Tag::Strikethrough), _)) => {}
            Some((Event::End(TagEnd::Strikethrough), _)) => {}
            Some((Event::Start(Tag::Link { dest_url, .. }), range)) => {
                let start = range.start;
                let (text, end) = collect_link_text_ranged(iter);
                spans.push(Span {
                    kind: SpanKind::Link {
                        text,
                        url: dest_url.into_string(),
                    },
                    source: SourceSpan::from_usize(start, end),
                });
            }
            Some((Event::SoftBreak, range)) => {
                spans.push(Span {
                    kind: SpanKind::SoftBreak,
                    source: SourceSpan::from_range(range),
                });
            }
            Some((Event::HardBreak, range)) => {
                spans.push(Span {
                    kind: SpanKind::HardBreak,
                    source: SourceSpan::from_range(range),
                });
            }
            None => break,
            _ => {}
        }
    }

    (spans, end_pos)
}

fn collect_link_text_ranged<'a, I>(iter: &mut I) -> (String, usize)
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    let mut text = String::new();
    let mut end_pos = 0;

    for (event, range) in iter {
        end_pos = range.end;
        match event {
            Event::End(TagEnd::Link) => break,
            Event::Text(t) => text.push_str(&t),
            _ => {}
        }
    }

    (text, end_pos)
}

fn collect_list_items_ranged<'a, I>(
    iter: &mut std::iter::Peekable<I>,
) -> (Vec<ListItem>, usize)
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    let mut items = Vec::new();
    let mut end_pos = 0;

    loop {
        match iter.next() {
            Some((Event::Start(Tag::Item), item_range)) => {
                let item_start = item_range.start;
                let mut spans = Vec::new();
                let mut nested = None;
                let mut item_end = item_range.end;
                let mut emphasis_level = 0u8; // Track bold/italic state

                loop {
                    match iter.next() {
                        Some((Event::End(TagEnd::Item), range)) => {
                            item_end = range.end;
                            break;
                        }
                        Some((Event::Text(t), range)) => {
                            let text = t.into_string();
                            let kind = match emphasis_level {
                                0 => SpanKind::Text(text),
                                1 => SpanKind::Emphasis(text),
                                2 => SpanKind::Strong(text),
                                _ => SpanKind::StrongEmphasis(text),
                            };
                            spans.push(Span {
                                kind,
                                source: SourceSpan::from_range(range),
                            });
                        }
                        Some((Event::Code(t), range)) => {
                            spans.push(Span {
                                kind: SpanKind::Code(t.into_string()),
                                source: SourceSpan::from_range(range),
                            });
                        }
                        Some((Event::Start(Tag::Emphasis), _)) => emphasis_level |= 1,
                        Some((Event::End(TagEnd::Emphasis), _)) => emphasis_level &= !1,
                        Some((Event::Start(Tag::Strong), _)) => emphasis_level |= 2,
                        Some((Event::End(TagEnd::Strong), _)) => emphasis_level &= !2,
                        Some((Event::Start(Tag::Paragraph), _)) => {
                            let (para_spans, _) = collect_spans_until_end_ranged(iter, TagEnd::Paragraph);
                            spans.extend(para_spans);
                        }
                        Some((Event::Start(Tag::List(start)), range)) => {
                            let list_start = range.start;
                            let (sub_items, list_end) = collect_list_items_ranged(iter);
                            nested = Some(Box::new(Element::List {
                                ordered: start.is_some(),
                                start,
                                items: sub_items,
                                source: SourceSpan::from_usize(list_start, list_end),
                            }));
                        }
                        Some((Event::SoftBreak, range)) => {
                            spans.push(Span {
                                kind: SpanKind::SoftBreak,
                                source: SourceSpan::from_range(range),
                            });
                        }
                        None => break,
                        _ => {}
                    }
                }

                items.push(ListItem {
                    spans,
                    nested,
                    source: SourceSpan::from_usize(item_start, item_end),
                });
            }
            Some((Event::End(TagEnd::List(_)), range)) => {
                end_pos = range.end;
                break;
            }
            None => break,
            _ => {}
        }
    }

    (items, end_pos)
}

fn collect_table_ranged<'a, I>(iter: &mut I) -> (Vec<String>, Vec<Vec<String>>, usize)
where
    I: Iterator<Item = RangedEvent<'a>>,
{
    let mut headers = Vec::new();
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_cell = String::new();
    let mut in_header = false;
    let mut end_pos = 0;

    for (event, range) in iter {
        end_pos = range.end;
        match event {
            Event::Start(Tag::TableHead) => in_header = true,
            Event::End(TagEnd::TableHead) => {
                if !current_cell.is_empty() {
                    headers.push(current_cell.clone());
                    current_cell.clear();
                }
                in_header = false;
            }
            Event::Start(Tag::TableRow) => current_row.clear(),
            Event::End(TagEnd::TableRow) => {
                if !in_header && !current_row.is_empty() {
                    rows.push(current_row.clone());
                }
            }
            Event::Start(Tag::TableCell) => current_cell.clear(),
            Event::End(TagEnd::TableCell) => {
                if in_header {
                    headers.push(current_cell.clone());
                } else {
                    current_row.push(current_cell.clone());
                }
                current_cell.clear();
            }
            Event::Text(t) => current_cell.push_str(&t),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }

    (headers, rows, end_pos)
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY COMPATIBILITY - for code that doesn't need positions yet
// ═══════════════════════════════════════════════════════════════════════════

impl Element {
    /// Get the text content of a heading (legacy helper)
    pub fn heading_text(&self) -> Option<&str> {
        match self {
            Element::Heading { text, .. } => Some(text),
            _ => None,
        }
    }
}
