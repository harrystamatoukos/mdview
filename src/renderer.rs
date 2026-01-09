use crate::document::Document;
use crate::parser::{self, Element, Span, SpanKind};
use crate::position::{FormattingKind, FormattingSpan, LayoutMap, MappedChar};
use crate::theme::{
    Theme, OPTIMAL_WIDTH, LEFT_MARGIN, MIN_MARGIN, TOP_PADDING,
    HEADING_SPACING_MAJOR, HEADING_SPACING_MINOR, SECTION_SPACING,
    BLOCKQUOTE_WIDTH_REDUCTION, NESTED_LIST_INDENT,
    TABLE_LABEL_MAX_WIDTH, TABLE_CARD_THRESHOLD,
};
use ratatui::style::Style;
use ratatui::text::{Line, Span as TuiSpan, Text};

// ═══════════════════════════════════════════════════════════════════════════
// STYLED CHARACTER - Tracks char, source position, and styling through wrapping
// ═══════════════════════════════════════════════════════════════════════════

/// A character with its source position and style information
#[derive(Clone)]
struct StyledChar {
    ch: char,
    source_offset: Option<usize>,
    style: Style,
}

/// A segment of styled text with position tracking
struct StyledSegment {
    text: String,
    char_offsets: Vec<Option<usize>>,
    style: Style,
}

// ═══════════════════════════════════════════════════════════════════════════
// MAPPED LINE BUILDER - Builds visual line + position mapping simultaneously
// ═══════════════════════════════════════════════════════════════════════════

/// Builder for a single rendered line with position tracking
struct MappedLineBuilder {
    spans: Vec<TuiSpan<'static>>,
    chars: Vec<MappedChar>,
}

impl MappedLineBuilder {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            chars: Vec::new(),
        }
    }

    /// Add synthetic (no source position) content
    fn push_synthetic(&mut self, text: &str, style: ratatui::style::Style) {
        for ch in text.chars() {
            self.chars.push(MappedChar::synthetic(ch));
        }
        self.spans.push(TuiSpan::styled(text.to_string(), style));
    }

    /// Add synthetic content with raw (default) style
    fn push_synthetic_raw(&mut self, text: &str) {
        for ch in text.chars() {
            self.chars.push(MappedChar::synthetic(ch));
        }
        self.spans.push(TuiSpan::raw(text.to_string()));
    }

    /// Add content with source position tracking
    fn push_mapped(&mut self, text: &str, start_offset: usize, style: ratatui::style::Style) {
        let mut offset = start_offset;
        for ch in text.chars() {
            self.chars.push(MappedChar::with_source(ch, offset));
            offset += ch.len_utf8();
        }
        self.spans.push(TuiSpan::styled(text.to_string(), style));
    }

    /// Finish and return (Line, MappedChars)
    fn finish(self) -> (Line<'static>, Vec<MappedChar>) {
        (Line::from(self.spans), self.chars)
    }
}

/// Render markdown content to plain text (for --print mode)
pub fn render(content: &str) -> String {
    let elements = parser::parse(content);
    let mut output = String::new();

    for element in elements {
        output.push_str(&render_element_plain(&element, 0));
    }

    output
}

/// Render markdown to ratatui Text WITH position mapping for WYSIWYG editing
/// Returns both the visual Text and a LayoutMap for cursor navigation
///
/// Takes a Document reference, using its pre-parsed elements to avoid redundant parsing.
/// The Document will parse automatically on first access if needed.
pub fn render_to_text(document: &mut Document, terminal_width: u16, theme: &Theme) -> (Text<'static>, LayoutMap) {
    // Get source_len before mutable borrow for elements
    let source_len = document.source_len();
    let elements = document.elements();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut layout_map = LayoutMap::new(source_len);

    // Calculate centering: how much left margin to add
    let content_width = OPTIMAL_WIDTH.min(terminal_width as usize - LEFT_MARGIN);
    let left_margin = if terminal_width as usize > content_width + LEFT_MARGIN {
        (terminal_width as usize - content_width) / 2
    } else {
        MIN_MARGIN
    };
    let margin = " ".repeat(left_margin);

    // Store margin in layout map for cursor positioning on empty lines
    layout_map.set_content_margin(left_margin);

    // Top padding - like a book page (synthetic - no source positions)
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
        layout_map.push_line(Vec::new());
    }

    for element in elements {
        render_element_mapped(&element, &mut lines, &mut layout_map, &margin, content_width, theme);
    }

    // End of document marker - synthetic
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
        layout_map.push_line(Vec::new());
    }
    let end_marker_margin = " ".repeat(left_margin + content_width / 2 - 4);
    let mut builder = MappedLineBuilder::new();
    builder.push_synthetic_raw(&end_marker_margin);
    builder.push_synthetic("· · ·", theme.hr());
    let (line, chars) = builder.finish();
    lines.push(line);
    layout_map.push_line(chars);

    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
        layout_map.push_line(Vec::new());
    }

    // Build index for fast lookups
    layout_map.build_index();

    (Text::from(lines), layout_map)
}

/// Render element with position mapping
fn render_element_mapped(
    element: &Element,
    lines: &mut Vec<Line<'static>>,
    layout_map: &mut LayoutMap,
    margin: &str,
    width: usize,
    theme: &Theme,
) {
    match element {
        Element::Heading { level, spans, .. } => {
            // Spacing above (synthetic)
            let spacing_above = if *level <= 2 { HEADING_SPACING_MAJOR } else { HEADING_SPACING_MINOR };
            for _ in 0..spacing_above {
                lines.push(Line::from(""));
                layout_map.push_line(Vec::new());
            }

            let style = match level {
                1 => theme.h1(),
                2 => theme.h2(),
                3 => theme.h3(),
                4 => theme.h4(),
                5 => theme.h5(),
                _ => theme.h6(),
            };

            // Render spans with position tracking; heading style overrides inline styles
            let (wrapped_lines, formatting_spans) = wrap_spans_styled(spans, width, theme);

            for fs in formatting_spans {
                layout_map.push_formatting_span(fs);
            }

            for segments in wrapped_lines {
                let mut builder = MappedLineBuilder::new();
                builder.push_synthetic_raw(margin);

                for segment in segments {
                    let text = if *level == 1 {
                        segment.text.to_ascii_uppercase()
                    } else {
                        segment.text.clone()
                    };

                    for (ch, maybe_offset) in text.chars().zip(segment.char_offsets.iter()) {
                        if let Some(offset) = maybe_offset {
                            builder.chars.push(MappedChar::with_source(ch, *offset));
                        } else {
                            builder.chars.push(MappedChar::synthetic(ch));
                        }
                    }
                    builder.spans.push(TuiSpan::styled(text, style));
                }

                let (line, chars) = builder.finish();
                lines.push(line);
                layout_map.push_line(chars);
            }

            // Spacing below (synthetic)
            if *level == 1 {
                lines.push(Line::from(""));
                layout_map.push_line(Vec::new());
            }
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        Element::Paragraph { spans, .. } => {
            // Render spans with position tracking, word wrap, AND style preservation
            let (wrapped_lines, formatting_spans) = wrap_spans_styled(spans, width, theme);

            // Register formatting spans for boundary-aware editing
            for fs in formatting_spans {
                layout_map.push_formatting_span(fs);
            }

            for segments in wrapped_lines {
                let mut builder = MappedLineBuilder::new();
                builder.push_synthetic_raw(margin);

                // Add each segment with its style and position mapping
                for segment in segments {
                    for (ch, maybe_offset) in segment.text.chars().zip(segment.char_offsets.iter()) {
                        if let Some(offset) = maybe_offset {
                            builder.chars.push(MappedChar::with_source(ch, *offset));
                        } else {
                            builder.chars.push(MappedChar::synthetic(ch));
                        }
                    }
                    builder.spans.push(TuiSpan::styled(segment.text, segment.style));
                }

                let (line, chars) = builder.finish();
                lines.push(line);
                layout_map.push_line(chars);
            }

            // Paragraph spacing
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        Element::CodeBlock { language, code, source } => {
            // Blank line before code block
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());

            // Calculate where actual code content starts (after opening fence line)
            // Fenced code blocks: "```" + language (if any) + "\n"
            let fence_line_len = 3 + language.as_ref().map(|l| l.len()).unwrap_or(0) + 1;
            let mut current_offset = source.start.get() + fence_line_len;

            // Code content - just indentation, no borders or boxes
            // Per DESIGN_PRINCIPLES: whitespace and indentation provide structure
            for code_line in code.lines() {
                let mut builder = MappedLineBuilder::new();
                builder.push_synthetic_raw(margin);
                builder.push_synthetic("        ", theme.body()); // 8-space indent for code
                builder.push_mapped(code_line, current_offset, theme.code_block());
                let (line, chars) = builder.finish();
                lines.push(line);
                layout_map.push_line(chars);

                current_offset += code_line.len() + 1; // +1 for newline
            }

            // Blank line after code block
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        Element::BlockQuote { elements, .. } => {
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());

            // Recursively render blockquote content
            for el in elements {
                let mut quote_lines: Vec<Line<'static>> = Vec::new();
                let mut quote_map = LayoutMap::new(0);
                render_element_mapped(el, &mut quote_lines, &mut quote_map, "", width.saturating_sub(BLOCKQUOTE_WIDTH_REDUCTION), theme);

                // Add blockquote border to each line
                for (i, line) in quote_lines.into_iter().enumerate() {
                    let is_empty = line.spans.iter().all(|s| s.content.trim().is_empty());
                    let quote_chars = quote_map.line(i).map(|s| s.to_vec()).unwrap_or_default();

                    let mut builder = MappedLineBuilder::new();
                    builder.push_synthetic_raw(margin);

                    if is_empty {
                        builder.push_synthetic("    │", theme.blockquote_border());
                    } else {
                        builder.push_synthetic("    │ ", theme.blockquote_border());
                        // Transfer the mapped chars from the nested rendering
                        builder.chars.extend(quote_chars);
                        // Preserve the original spans with their formatting (bold, italic, etc.)
                        builder.spans.extend(line.spans);
                    }
                    let (line, chars) = builder.finish();
                    lines.push(line);
                    layout_map.push_line(chars);
                }
            }

            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        Element::List { ordered, start, items, .. } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("  {}. ", start.unwrap_or(1) + i as u64)
                } else {
                    "  •  ".to_string()
                };

                let marker_width = marker.chars().count();
                let text_width = width.saturating_sub(marker_width);

                // Use wrap_spans_styled to preserve source positions AND styles
                let (wrapped_lines, formatting_spans) = wrap_spans_styled(&item.spans, text_width, theme);

                // Register formatting spans for boundary-aware editing
                for fs in formatting_spans {
                    layout_map.push_formatting_span(fs);
                }

                for (j, segments) in wrapped_lines.iter().enumerate() {
                    let mut builder = MappedLineBuilder::new();
                    builder.push_synthetic_raw(margin);

                    if j == 0 {
                        builder.push_synthetic(&marker, theme.list_marker());
                    } else {
                        builder.push_synthetic_raw(&" ".repeat(marker_width));
                    }

                    // Add each segment with its style and position mapping
                    for segment in segments {
                        for (ch, maybe_offset) in segment.text.chars().zip(segment.char_offsets.iter()) {
                            if let Some(offset) = maybe_offset {
                                builder.chars.push(MappedChar::with_source(ch, *offset));
                            } else {
                                builder.chars.push(MappedChar::synthetic(ch));
                            }
                        }
                        builder.spans.push(TuiSpan::styled(segment.text.clone(), segment.style));
                    }

                    let (line, chars) = builder.finish();
                    lines.push(line);
                    layout_map.push_line(chars);
                }

                // Handle nested lists recursively
                if let Some(nested) = &item.nested {
                    let nested_margin = format!("{}{}", margin, " ".repeat(NESTED_LIST_INDENT));
                    render_element_mapped(nested, lines, layout_map, &nested_margin, width.saturating_sub(NESTED_LIST_INDENT), theme);
                }
            }
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        Element::HorizontalRule { .. } => {
            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
                layout_map.push_line(Vec::new());
            }

            let rule_margin = " ".repeat(margin.len() + width / 4);
            let mut builder = MappedLineBuilder::new();
            builder.push_synthetic_raw(&rule_margin);
            builder.push_synthetic("─  ·  ─", theme.hr());
            let (line, chars) = builder.finish();
            lines.push(line);
            layout_map.push_line(chars);

            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
                layout_map.push_line(Vec::new());
            }
        }

        Element::Table { headers, rows, .. } => {
            // For now, tables are rendered without fine-grained position mapping
            // (complex due to column alignment) - treat as mostly synthetic
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());

            let natural_widths: Vec<usize> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let header_len = h.chars().count();
                    let max_row = rows
                        .iter()
                        .map(|r| r.get(i).map(|c| c.chars().count()).unwrap_or(0))
                        .max()
                        .unwrap_or(0);
                    header_len.max(max_row)
                })
                .collect();

            let natural_total: usize = natural_widths.iter().sum::<usize>() + (headers.len() - 1) * 3 + 4;

            if natural_total <= width {
                render_table_tabular_mapped(lines, layout_map, margin, headers, rows, &natural_widths, theme);
            } else {
                render_table_cards_mapped(lines, layout_map, margin, headers, rows, width, theme);
            }

            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }
    }
}

/// Word wrap spans while preserving source positions (including spaces)
/// NOTE: Deprecated in favor of wrap_spans_styled which preserves inline formatting
#[allow(dead_code)]
fn wrap_spans_mapped(spans: &[Span], width: usize, _base_offset: usize) -> Vec<(String, Vec<Option<usize>>)> {
    if width == 0 {
        return vec![(String::new(), Vec::new())];
    }

    let mut result: Vec<(String, Vec<Option<usize>>)> = Vec::new();
    let mut current_line = String::new();
    let mut current_offsets: Vec<Option<usize>> = Vec::new();
    let mut current_len = 0;

    // Collect all characters with their source offsets first
    let mut all_chars: Vec<(char, Option<usize>)> = Vec::new();

    for span in spans {
        let span_start = span.source.start.get();
        let (text, is_synthetic) = match &span.kind {
            SpanKind::Text(t) => (t.clone(), false),
            SpanKind::Emphasis(t) => (t.clone(), false),
            SpanKind::Strong(t) => (t.clone(), false),
            SpanKind::StrongEmphasis(t) => (t.clone(), false),
            SpanKind::Code(t) => (format!("‹{}›", t), true), // Brackets are synthetic
            SpanKind::Link { text, url } => (format!("{} [→ {}]", text, url), true),
            SpanKind::Strikethrough(t) => (t.clone(), false),
            SpanKind::SoftBreak => (" ".to_string(), false),
            SpanKind::HardBreak => ("\n".to_string(), false),
        };

        if is_synthetic {
            // For synthetic content (like code brackets), map all chars to span start
            for ch in text.chars() {
                all_chars.push((ch, Some(span_start)));
            }
        } else {
            // Map each character to its source position
            let mut byte_offset = 0;
            for ch in text.chars() {
                all_chars.push((ch, Some(span_start + byte_offset)));
                byte_offset += ch.len_utf8();
            }
        }
    }

    // Now do word wrapping while preserving positions
    let mut i = 0;

    while i < all_chars.len() {
        let (ch, offset) = all_chars[i];

        if ch == '\n' {
            // Hard break - start new line
            result.push((current_line, current_offsets));
            current_line = String::new();
            current_offsets = Vec::new();
            current_len = 0;
            i += 1;
            continue;
        }

        if ch.is_whitespace() {
            // Handle whitespace - check if we need to wrap before adding
            if current_len > 0 {
                // Look ahead to see the next word length
                let next_word_len = count_next_word_len(&all_chars, i + 1);

                if current_len + 1 + next_word_len > width && next_word_len > 0 {
                    // Wrap before adding space
                    result.push((current_line, current_offsets));
                    current_line = String::new();
                    current_offsets = Vec::new();
                    current_len = 0;
                    // Skip the space at line break
                    i += 1;
                    continue;
                }

                // Add the space with its source mapping
                current_line.push(' ');
                current_offsets.push(offset);
                current_len += 1;
            }
            i += 1;
        } else {
            // Regular character
            current_line.push(ch);
            current_offsets.push(offset);
            current_len += 1;
            i += 1;
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push((current_line, current_offsets));
    }

    result
}

/// Count the length of the next word (non-whitespace sequence)
#[allow(dead_code)]
fn count_next_word_len(chars: &[(char, Option<usize>)], start: usize) -> usize {
    let mut len = 0;
    for i in start..chars.len() {
        let (ch, _) = chars[i];
        if ch.is_whitespace() {
            break;
        }
        len += 1;
    }
    len
}

/// Count the length of the next word in styled chars
fn count_next_word_len_styled(chars: &[StyledChar], start: usize) -> usize {
    let mut len = 0;
    for i in start..chars.len() {
        if chars[i].ch.is_whitespace() {
            break;
        }
        len += 1;
    }
    len
}

/// Group consecutive styled chars with the same style into segments
fn group_styled_chars_into_segments(chars: &[StyledChar]) -> Vec<StyledSegment> {
    if chars.is_empty() {
        return Vec::new();
    }

    // Typically few style changes per line, estimate ~4 segments max
    let mut segments = Vec::with_capacity(4);
    let mut current_text = String::with_capacity(chars.len());
    let mut current_offsets: Vec<Option<usize>> = Vec::with_capacity(chars.len());
    let mut current_style = chars[0].style;

    for sc in chars {
        if sc.style == current_style {
            current_text.push(sc.ch);
            current_offsets.push(sc.source_offset);
        } else {
            // Style changed - finish current segment
            if !current_text.is_empty() {
                segments.push(StyledSegment {
                    text: current_text,
                    char_offsets: current_offsets,
                    style: current_style,
                });
            }
            // Start new segment
            current_text = sc.ch.to_string();
            current_offsets = vec![sc.source_offset];
            current_style = sc.style;
        }
    }

    // Don't forget the last segment
    if !current_text.is_empty() {
        segments.push(StyledSegment {
            text: current_text,
            char_offsets: current_offsets,
            style: current_style,
        });
    }

    segments
}

/// Word wrap spans while preserving source positions AND styles
/// Returns: (wrapped lines as styled segments, formatting spans for boundary detection)
fn wrap_spans_styled(spans: &[Span], width: usize, theme: &Theme) -> (Vec<Vec<StyledSegment>>, Vec<FormattingSpan>) {
    if width == 0 {
        return (vec![vec![StyledSegment {
            text: String::new(),
            char_offsets: Vec::new(),
            style: Style::default(),
        }]], Vec::new());
    }

    // Estimate capacity from source spans to reduce reallocations
    let estimated_chars: usize = spans.iter().map(|s| s.source.end - s.source.start).sum();
    let mut all_chars: Vec<StyledChar> = Vec::with_capacity(estimated_chars);
    // Most spans won't have formatting, but pre-allocate a small amount
    let mut formatting_spans: Vec<FormattingSpan> = Vec::with_capacity(spans.len() / 4);

    for span in spans {
        let span_start = span.source.start.get();
        let span_end = span.source.end.get();
        let (text, style, formatting_kind) = match &span.kind {
            SpanKind::Text(t) => (t.clone(), theme.body(), None),
            SpanKind::Emphasis(t) => (t.clone(), theme.emphasis(), Some(FormattingKind::Emphasis)),
            SpanKind::Strong(t) => (t.clone(), theme.strong(), Some(FormattingKind::Strong)),
            SpanKind::StrongEmphasis(t) => (t.clone(), theme.strong_emphasis(), Some(FormattingKind::StrongEmphasis)),
            SpanKind::Code(t) => (t.clone(), theme.inline_code(), Some(FormattingKind::Code)),
            SpanKind::Link { text, url } => {
                // Links: styled text + dim URL
                let link_text = format!("{} [→ {}]", text, url);
                (link_text, theme.link(), None) // Links don't have simple markers
            }
            SpanKind::Strikethrough(t) => (t.clone(), theme.strikethrough(), Some(FormattingKind::Strikethrough)),
            SpanKind::SoftBreak => (" ".to_string(), theme.body(), None),
            SpanKind::HardBreak => ("\n".to_string(), theme.body(), None),
        };

        // Track formatting spans for boundary-aware editing
        if let Some(kind) = formatting_kind {
            formatting_spans.push(FormattingSpan::from_content_range(
                kind,
                span_start,
                span_end,
            ));
        }

        // For links, all chars map to span start (synthetic-ish)
        let is_link = matches!(&span.kind, SpanKind::Link { .. });

        // Map each character to its source position
        let mut byte_offset = 0;
        for ch in text.chars() {
            let source_offset = if is_link {
                Some(span_start) // Links map all to start
            } else {
                Some(span_start + byte_offset)
            };
            all_chars.push(StyledChar {
                ch,
                source_offset,
                style,
            });
            byte_offset += ch.len_utf8();
        }
    }

    // Now do word wrapping while preserving styles
    // Estimate lines based on total chars / width
    let estimated_lines = (all_chars.len() / width.max(1)).max(1);
    let mut result: Vec<Vec<StyledSegment>> = Vec::with_capacity(estimated_lines);
    let mut current_line: Vec<StyledChar> = Vec::with_capacity(width);
    let mut current_len = 0;
    let mut i = 0;

    while i < all_chars.len() {
        let sc = &all_chars[i];

        if sc.ch == '\n' {
            // Hard break - start new line
            result.push(group_styled_chars_into_segments(&current_line));
            current_line = Vec::with_capacity(width);
            current_len = 0;
            i += 1;
            continue;
        }

        if sc.ch.is_whitespace() {
            // Handle whitespace - check if we need to wrap before adding
            if current_len > 0 {
                // Look ahead to see the next word length
                let next_word_len = count_next_word_len_styled(&all_chars, i + 1);

                if current_len + 1 + next_word_len > width && next_word_len > 0 {
                    // Wrap before adding space
                    result.push(group_styled_chars_into_segments(&current_line));
                    current_line = Vec::with_capacity(width);
                    current_len = 0;
                    // Skip the space at line break
                    i += 1;
                    continue;
                }

                // Add the space
                current_line.push(sc.clone());
                current_len += 1;
            }
            i += 1;
        } else {
            // Regular character
            current_line.push(sc.clone());
            current_len += 1;
            i += 1;
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(group_styled_chars_into_segments(&current_line));
    }

    (result, formatting_spans)
}

/// Render table in tabular format with position mapping
fn render_table_tabular_mapped(
    lines: &mut Vec<Line<'static>>,
    layout_map: &mut LayoutMap,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    col_widths: &[usize],
    theme: &Theme,
) {
    let total_width: usize = col_widths.iter().sum::<usize>() + (headers.len() - 1) * 3 + 4;

    // Top border
    let mut builder = MappedLineBuilder::new();
    builder.push_synthetic_raw(margin);
    builder.push_synthetic(&format!("  ┌{}┐", "─".repeat(total_width - 2)), theme.table_border());
    let (line, chars) = builder.finish();
    lines.push(line);
    layout_map.push_line(chars);

    // Header row
    let mut builder = MappedLineBuilder::new();
    builder.push_synthetic_raw(margin);
    builder.push_synthetic("  │", theme.table_border());
    for (i, h) in headers.iter().enumerate() {
        let w = col_widths[i];
        builder.push_synthetic(&format!(" {:<width$}", h, width = w), theme.table_header());
        if i < headers.len() - 1 {
            builder.push_synthetic(" │", theme.table_border());
        }
    }
    builder.push_synthetic(" │", theme.table_border());
    let (line, chars) = builder.finish();
    lines.push(line);
    layout_map.push_line(chars);

    // Separator
    let sep_parts: Vec<String> = col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
    let mut builder = MappedLineBuilder::new();
    builder.push_synthetic_raw(margin);
    builder.push_synthetic(&format!("  ├{}┤", sep_parts.join("┼")), theme.table_border());
    let (line, chars) = builder.finish();
    lines.push(line);
    layout_map.push_line(chars);

    // Data rows
    for row in rows {
        let mut builder = MappedLineBuilder::new();
        builder.push_synthetic_raw(margin);
        builder.push_synthetic("  │", theme.table_border());
        for (i, c) in row.iter().enumerate() {
            let w = col_widths.get(i).copied().unwrap_or(10);
            builder.push_synthetic(&format!(" {:<width$}", c, width = w), theme.body());
            if i < row.len() - 1 {
                builder.push_synthetic(" │", theme.table_border());
            }
        }
        builder.push_synthetic(" │", theme.table_border());
        let (line, chars) = builder.finish();
        lines.push(line);
        layout_map.push_line(chars);
    }

    // Bottom border
    let mut builder = MappedLineBuilder::new();
    builder.push_synthetic_raw(margin);
    builder.push_synthetic(&format!("  └{}┘", "─".repeat(total_width - 2)), theme.table_border());
    let (line, chars) = builder.finish();
    lines.push(line);
    layout_map.push_line(chars);
}

/// Render table as cards with position mapping
fn render_table_cards_mapped(
    lines: &mut Vec<Line<'static>>,
    layout_map: &mut LayoutMap,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    width: usize,
    theme: &Theme,
) {
    let max_header_len = headers.iter().map(|h| h.chars().count()).max().unwrap_or(10);
    let label_width = max_header_len.min(TABLE_LABEL_MAX_WIDTH);
    let value_width = width.saturating_sub(label_width + BLOCKQUOTE_WIDTH_REDUCTION);

    for (row_idx, row) in rows.iter().enumerate() {
        if row_idx > 0 {
            lines.push(Line::from(""));
            layout_map.push_line(Vec::new());
        }

        // Card header
        let separator_width = width.saturating_sub(8);
        let mut builder = MappedLineBuilder::new();
        builder.push_synthetic_raw(margin);
        builder.push_synthetic(&format!("  ─── {} ", row_idx + 1), theme.table_border());
        builder.push_synthetic(&"─".repeat(separator_width), theme.table_border());
        let (line, chars) = builder.finish();
        lines.push(line);
        layout_map.push_line(chars);

        // Fields
        for (i, header) in headers.iter().enumerate() {
            let value = row.get(i).map(|s| s.as_str()).unwrap_or("");
            let value_lines = wrap_text(value, value_width);

            for (line_idx, value_line) in value_lines.iter().enumerate() {
                let mut builder = MappedLineBuilder::new();
                builder.push_synthetic_raw(margin);
                if line_idx == 0 {
                    builder.push_synthetic(&format!("  {:<width$}", header, width = label_width), theme.table_header());
                    builder.push_synthetic("  ", theme.body());
                } else {
                    builder.push_synthetic_raw(&" ".repeat(label_width + 4));
                }
                builder.push_synthetic(value_line, theme.body());
                let (line, chars) = builder.finish();
                lines.push(line);
                layout_map.push_line(chars);
            }
        }
    }
}

/// Render table in traditional tabular format (for tables that fit)
fn render_table_tabular(
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    col_widths: &[usize],
    theme: &Theme,
) {
    let total_width: usize = col_widths.iter().sum::<usize>() + (headers.len() - 1) * 3 + 4;

    // Top border
    lines.push(Line::from(vec![
        TuiSpan::raw(margin.to_string()),
        TuiSpan::styled(format!("  ┌{}┐", "─".repeat(total_width - 2)), theme.table_border()),
    ]));

    // Header row
    let header_cells: Vec<TuiSpan> = headers
        .iter()
        .enumerate()
        .flat_map(|(i, h)| {
            let w = col_widths[i];
            let mut spans = vec![
                TuiSpan::styled(format!(" {:<width$}", h, width = w), theme.table_header()),
            ];
            if i < headers.len() - 1 {
                spans.push(TuiSpan::styled(" │", theme.table_border()));
            }
            spans
        })
        .collect();

    let mut header_line = vec![
        TuiSpan::raw(margin.to_string()),
        TuiSpan::styled("  │", theme.table_border()),
    ];
    header_line.extend(header_cells);
    header_line.push(TuiSpan::styled(" │", theme.table_border()));
    lines.push(Line::from(header_line));

    // Separator
    let sep_parts: Vec<String> = col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
    lines.push(Line::from(vec![
        TuiSpan::raw(margin.to_string()),
        TuiSpan::styled(format!("  ├{}┤", sep_parts.join("┼")), theme.table_border()),
    ]));

    // Data rows
    for row in rows {
        let row_cells: Vec<TuiSpan> = row
            .iter()
            .enumerate()
            .flat_map(|(i, c)| {
                let w = col_widths.get(i).copied().unwrap_or(10);
                let mut spans = vec![
                    TuiSpan::styled(format!(" {:<width$}", c, width = w), theme.body()),
                ];
                if i < row.len() - 1 {
                    spans.push(TuiSpan::styled(" │", theme.table_border()));
                }
                spans
            })
            .collect();

        let mut row_line = vec![
            TuiSpan::raw(margin.to_string()),
            TuiSpan::styled("  │", theme.table_border()),
        ];
        row_line.extend(row_cells);
        row_line.push(TuiSpan::styled(" │", theme.table_border()));
        lines.push(Line::from(row_line));
    }

    // Bottom border
    lines.push(Line::from(vec![
        TuiSpan::raw(margin.to_string()),
        TuiSpan::styled(format!("  └{}┘", "─".repeat(total_width - 2)), theme.table_border()),
    ]));
}

/// Render table as cards (for tables too wide to fit)
fn render_table_cards(
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    width: usize,
    theme: &Theme,
) {
    // Find the longest header for alignment
    let max_header_len = headers.iter().map(|h| h.chars().count()).max().unwrap_or(10);
    let label_width = max_header_len.min(TABLE_LABEL_MAX_WIDTH);
    let value_width = width.saturating_sub(label_width + BLOCKQUOTE_WIDTH_REDUCTION);

    for (row_idx, row) in rows.iter().enumerate() {
        // Card separator
        if row_idx > 0 {
            lines.push(Line::from(""));
        }

        // Top of card - minimal separator with row number
        let separator_width = width.saturating_sub(8);
        lines.push(Line::from(vec![
            TuiSpan::raw(margin.to_string()),
            TuiSpan::styled(format!("  ─── {} ", row_idx + 1), theme.table_border()),
            TuiSpan::styled("─".repeat(separator_width), theme.table_border()),
        ]));

        // Each field as a row
        for (i, header) in headers.iter().enumerate() {
            let value = row.get(i).map(|s| s.as_str()).unwrap_or("");

            // Wrap long values
            let value_lines = wrap_text(value, value_width);

            for (line_idx, value_line) in value_lines.iter().enumerate() {
                if line_idx == 0 {
                    // First line shows the label
                    lines.push(Line::from(vec![
                        TuiSpan::raw(margin.to_string()),
                        TuiSpan::styled(format!("  {:<width$}", header, width = label_width), theme.table_header()),
                        TuiSpan::styled("  ", theme.body()),
                        TuiSpan::styled(value_line.clone(), theme.body()),
                    ]));
                } else {
                    // Continuation lines are indented
                    lines.push(Line::from(vec![
                        TuiSpan::raw(margin.to_string()),
                        TuiSpan::raw(" ".repeat(label_width + 4)),
                        TuiSpan::styled(value_line.clone(), theme.body()),
                    ]));
                }
            }
        }
    }
}

fn render_spans_to_string(spans: &[Span]) -> String {
    use crate::parser::SpanKind;
    spans
        .iter()
        .map(|span| match &span.kind {
            SpanKind::Text(t) => t.clone(),
            SpanKind::Emphasis(t) => t.clone(),
            SpanKind::Strong(t) => t.clone(),
            SpanKind::StrongEmphasis(t) => t.clone(),
            SpanKind::Code(t) => format!("‹{}›", t),  // Subtle code markers
            SpanKind::Link { text, url } => format!("{} [→ {}]", text, url),
            SpanKind::Strikethrough(t) => t.clone(),
            SpanKind::SoftBreak => " ".to_string(),
            SpanKind::HardBreak => "\n".to_string(),
        })
        .collect()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line = word.to_string();
        } else if current_line.chars().count() + 1 + word.chars().count() <= width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(current_line);
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

// ─────────────────────────────────────────────────────────────
// Plain text rendering (for --print mode)
// ─────────────────────────────────────────────────────────────

fn render_element_plain(element: &Element, indent: usize) -> String {
    let margin = " ".repeat(indent + LEFT_MARGIN);

    match element {
        Element::Heading { level, text, .. } => {
            let spacing = if *level <= 2 {
                "\n".repeat(HEADING_SPACING_MAJOR)
            } else {
                "\n".repeat(HEADING_SPACING_MINOR)
            };
            if *level == 1 {
                format!("{}{}{}\n\n", spacing, margin, text.to_ascii_uppercase())
            } else {
                format!("{}{}{}\n\n", spacing, margin, text)
            }
        }
        Element::Paragraph { spans, .. } => {
            let text = render_spans_to_string(spans);
            let wrapped: Vec<String> = wrap_text(&text, OPTIMAL_WIDTH)
                .iter()
                .map(|l| format!("{}{}", margin, l))
                .collect();
            format!("{}\n\n", wrapped.join("\n"))
        }
        Element::CodeBlock { language: _, code, .. } => {
            let mut output = String::from("\n");
            // Code content - just indentation, no borders
            for line in code.lines() {
                output.push_str(&format!("{}        {}\n", margin, line));
            }
            output.push('\n');
            output
        }
        Element::BlockQuote { elements, .. } => {
            let mut output = String::from("\n");
            for el in elements {
                let rendered = render_element_plain(el, indent);
                for line in rendered.lines() {
                    output.push_str(&format!("{}    │ {}\n", margin, line.trim()));
                }
            }
            output.push('\n');
            output
        }
        Element::List { ordered, start, items, .. } => {
            let mut output = String::new();
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", start.unwrap_or(1) + i as u64)
                } else {
                    "•  ".to_string()
                };
                let text = render_spans_to_string(&item.spans);
                output.push_str(&format!("{}  {}{}\n", margin, marker, text));

                if let Some(nested) = &item.nested {
                    output.push_str(&render_element_plain(nested, indent + 3));
                }
            }
            output.push('\n');
            output
        }
        Element::HorizontalRule { .. } => {
            let hr_margin = " ".repeat(indent + OPTIMAL_WIDTH / 4);
            format!(
                "{}\n{}─  ·  ─\n{}\n",
                "\n".repeat(SECTION_SPACING),
                hr_margin,
                "\n".repeat(SECTION_SPACING)
            )
        }
        Element::Table { headers, rows, .. } => {
            let mut output = String::from("\n");

            // Calculate natural column widths
            let col_widths: Vec<usize> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let header_len = h.chars().count();
                    let max_row = rows
                        .iter()
                        .map(|r| r.get(i).map(|c| c.chars().count()).unwrap_or(0))
                        .max()
                        .unwrap_or(0);
                    header_len.max(max_row)
                })
                .collect();

            let total_width: usize = col_widths.iter().sum::<usize>() + (headers.len() - 1) * 3 + 4;

            // If table fits in threshold, use tabular; otherwise use cards
            if total_width <= TABLE_CARD_THRESHOLD {
                // TABULAR
                output.push_str(&format!("{}┌{}┐\n", margin, "─".repeat(total_width - 2)));

                let header_cells: Vec<String> = headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| format!(" {:<width$} ", h, width = col_widths[i]))
                    .collect();
                output.push_str(&format!("{}│{}│\n", margin, header_cells.join("│")));

                let sep_parts: Vec<String> = col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
                output.push_str(&format!("{}├{}┤\n", margin, sep_parts.join("┼")));

                for row in rows {
                    let row_cells: Vec<String> = row
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let w = col_widths.get(i).copied().unwrap_or(10);
                            format!(" {:<width$} ", c, width = w)
                        })
                        .collect();
                    output.push_str(&format!("{}│{}│\n", margin, row_cells.join("│")));
                }

                output.push_str(&format!("{}└{}┘\n", margin, "─".repeat(total_width - 2)));
            } else {
                // CARD LAYOUT
                let max_header_len = headers.iter().map(|h| h.chars().count()).max().unwrap_or(10);
                let label_width = max_header_len.min(TABLE_LABEL_MAX_WIDTH);
                let separator_width = OPTIMAL_WIDTH.saturating_sub(8);

                for (row_idx, row) in rows.iter().enumerate() {
                    if row_idx > 0 {
                        output.push('\n');
                    }
                    output.push_str(&format!("{}─── {} {}\n", margin, row_idx + 1, "─".repeat(separator_width)));

                    for (i, header) in headers.iter().enumerate() {
                        let value = row.get(i).map(|s| s.as_str()).unwrap_or("");
                        output.push_str(&format!("{}{:<width$}  {}\n", margin, header, value, width = label_width));
                    }
                }
            }

            output.push('\n');
            output
        }
    }
}
