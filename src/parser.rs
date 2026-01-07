use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, HeadingLevel, CodeBlockKind};

/// Parsed markdown element for rendering
#[derive(Debug, Clone)]
pub enum Element {
    Heading { level: u8, text: String },
    Paragraph { spans: Vec<Span> },
    CodeBlock { language: Option<String>, code: String },
    BlockQuote { elements: Vec<Element> },
    List { ordered: bool, start: Option<u64>, items: Vec<ListItem> },
    HorizontalRule,
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub spans: Vec<Span>,
    pub nested: Option<Box<Element>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Variants for future inline styling support
pub enum Span {
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

/// Parse markdown content into our intermediate representation
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

fn parse_event<'a, I>(event: Event<'a>, iter: &mut std::iter::Peekable<I>) -> Option<Element>
where
    I: Iterator<Item = Event<'a>>,
{
    match event {
        Event::Start(Tag::Heading { level, .. }) => {
            let text = collect_text_until_end(iter, TagEnd::Heading(level));
            Some(Element::Heading {
                level: heading_level_to_u8(level),
                text,
            })
        }
        Event::Start(Tag::Paragraph) => {
            let spans = collect_spans_until_end(iter, TagEnd::Paragraph);
            Some(Element::Paragraph { spans })
        }
        Event::Start(Tag::CodeBlock(kind)) => {
            let language = match kind {
                CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                _ => None,
            };
            let code = collect_text_until_end(iter, TagEnd::CodeBlock);
            Some(Element::CodeBlock { language, code })
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
            Some(Element::BlockQuote { elements: inner_elements })
        }
        Event::Start(Tag::List(start)) => {
            let ordered = start.is_some();
            let items = collect_list_items(iter);
            Some(Element::List { ordered, start, items })
        }
        Event::Rule => Some(Element::HorizontalRule),
        Event::Start(Tag::Table(_)) => {
            let (headers, rows) = collect_table(iter);
            Some(Element::Table { headers, rows })
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

fn collect_text_until_end<'a, I>(iter: &mut I, end_tag: TagEnd) -> String
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

fn collect_spans_until_end<'a, I>(iter: &mut std::iter::Peekable<I>, end_tag: TagEnd) -> Vec<Span>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut spans = Vec::new();
    let mut emphasis_level = 0u8; // Track nesting: 0=none, 1=emphasis, 2=strong, 3=both

    loop {
        match iter.next() {
            Some(Event::End(ref tag)) if *tag == end_tag => break,
            Some(Event::Text(t)) => {
                let span = match emphasis_level {
                    0 => Span::Text(t.to_string()),
                    1 => Span::Emphasis(t.to_string()),
                    2 => Span::Strong(t.to_string()),
                    _ => Span::StrongEmphasis(t.to_string()),
                };
                spans.push(span);
            }
            Some(Event::Code(t)) => spans.push(Span::Code(t.to_string())),
            Some(Event::Start(Tag::Emphasis)) => emphasis_level |= 1,
            Some(Event::End(TagEnd::Emphasis)) => emphasis_level &= !1,
            Some(Event::Start(Tag::Strong)) => emphasis_level |= 2,
            Some(Event::End(TagEnd::Strong)) => emphasis_level &= !2,
            Some(Event::Start(Tag::Strikethrough)) => {}
            Some(Event::End(TagEnd::Strikethrough)) => {}
            Some(Event::Start(Tag::Link { dest_url, .. })) => {
                let text = collect_link_text(iter);
                spans.push(Span::Link {
                    text,
                    url: dest_url.to_string(),
                });
            }
            Some(Event::SoftBreak) => spans.push(Span::SoftBreak),
            Some(Event::HardBreak) => spans.push(Span::HardBreak),
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

fn collect_list_items<'a, I>(iter: &mut std::iter::Peekable<I>) -> Vec<ListItem>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut items = Vec::new();

    loop {
        match iter.next() {
            Some(Event::Start(Tag::Item)) => {
                let mut spans = Vec::new();
                let mut nested = None;

                loop {
                    match iter.next() {
                        Some(Event::End(TagEnd::Item)) => break,
                        Some(Event::Text(t)) => spans.push(Span::Text(t.to_string())),
                        Some(Event::Code(t)) => spans.push(Span::Code(t.to_string())),
                        Some(Event::Start(Tag::Emphasis)) => {}
                        Some(Event::End(TagEnd::Emphasis)) => {}
                        Some(Event::Start(Tag::Strong)) => {}
                        Some(Event::End(TagEnd::Strong)) => {}
                        Some(Event::Start(Tag::Paragraph)) => {
                            // Inline paragraph in list item - collect its spans
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
                        Some(Event::SoftBreak) => spans.push(Span::SoftBreak),
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

    (headers, rows)
}
