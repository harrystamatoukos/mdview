use crate::parser::{self, CellAlign, Element, Span, SpanKind};
use crate::theme::{
    BLOCKQUOTE_WIDTH_REDUCTION, HEADING_SPACING_MAJOR, HEADING_SPACING_MINOR, LEFT_MARGIN,
    MIN_MARGIN, NESTED_LIST_INDENT, OPTIMAL_WIDTH, SECTION_SPACING, TABLE_CARD_THRESHOLD,
    TABLE_LABEL_MAX_WIDTH, TOP_PADDING, Theme,
};
use ratatui::style::Style;
use ratatui::text::{Line, Span as TuiSpan, Text};

// ═══════════════════════════════════════════════════════════════════════════
// LINE BUILDER - accumulates styled visual spans for one rendered line
// ═══════════════════════════════════════════════════════════════════════════

/// A run of text sharing one style (after inline-style grouping).
struct StyledSegment {
    text: String,
    style: Style,
}

/// A single styled character (used during word-wrapping).
#[derive(Clone)]
struct StyledChar {
    ch: char,
    style: Style,
}

/// Builder for a single rendered line.
struct LineBuilder {
    spans: Vec<TuiSpan<'static>>,
}

impl LineBuilder {
    fn new() -> Self {
        Self { spans: Vec::new() }
    }

    /// Add styled content.
    fn push(&mut self, text: &str, style: Style) {
        self.spans.push(TuiSpan::styled(text.to_string(), style));
    }

    /// Add content with the default (raw) style.
    fn push_raw(&mut self, text: &str) {
        self.spans.push(TuiSpan::raw(text.to_string()));
    }

    fn finish(self) -> Line<'static> {
        Line::from(self.spans)
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

/// Render markdown to a styled ratatui `Text` for the read-only TUI reader.
pub fn render_to_text(content: &str, terminal_width: u16, theme: &Theme) -> Text<'static> {
    let elements = parser::parse(content);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Calculate centering: how much left margin to add.
    // Keep the math saturating so tiny terminal widths do not underflow.
    let terminal_width = terminal_width as usize;
    let content_width = OPTIMAL_WIDTH
        .min(terminal_width.saturating_sub(LEFT_MARGIN))
        .max(1);
    let left_margin = if terminal_width > content_width + LEFT_MARGIN {
        (terminal_width - content_width) / 2
    } else {
        MIN_MARGIN.min(terminal_width.saturating_sub(content_width))
    };
    let margin = " ".repeat(left_margin);

    // Top padding - like a book page.
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }

    for element in &elements {
        render_element_styled(element, &mut lines, &margin, content_width, theme);
    }

    // End of document marker.
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }
    let end_marker_margin = " ".repeat((left_margin + content_width / 2).saturating_sub(4));
    let mut builder = LineBuilder::new();
    builder.push_raw(&end_marker_margin);
    builder.push("· · ·", theme.hr());
    lines.push(builder.finish());

    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }

    Text::from(lines)
}

/// Render one element to styled lines.
fn render_element_styled(
    element: &Element,
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    width: usize,
    theme: &Theme,
) {
    match element {
        Element::Heading { level, spans, .. } => {
            let spacing_above = if *level <= 2 {
                HEADING_SPACING_MAJOR
            } else {
                HEADING_SPACING_MINOR
            };
            for _ in 0..spacing_above {
                lines.push(Line::from(""));
            }

            let style = match level {
                1 => theme.h1(),
                2 => theme.h2(),
                3 => theme.h3(),
                4 => theme.h4(),
                5 => theme.h5(),
                _ => theme.h6(),
            };

            let wrapped_lines = wrap_spans_styled(spans, width, theme);

            for (line_idx, segments) in wrapped_lines.into_iter().enumerate() {
                let mut builder = LineBuilder::new();
                builder.push_raw(margin);

                // H1 gets a warm accent bar; continuation lines align under the title.
                if *level == 1 {
                    if line_idx == 0 {
                        builder.push("▌ ", theme.h1_accent());
                    } else {
                        builder.push_raw("  ");
                    }
                }

                for segment in segments {
                    let text = if *level == 1 {
                        segment.text.to_ascii_uppercase()
                    } else {
                        segment.text
                    };
                    builder.push(&text, style);
                }

                lines.push(builder.finish());
            }

            if *level == 1 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(""));
        }

        Element::Paragraph { spans, .. } => {
            let wrapped_lines = wrap_spans_styled(spans, width, theme);

            for segments in wrapped_lines {
                let mut builder = LineBuilder::new();
                builder.push_raw(margin);
                for segment in segments {
                    builder.push(&segment.text, segment.style);
                }
                lines.push(builder.finish());
            }

            lines.push(Line::from(""));
        }

        Element::CodeBlock { code, lang } => {
            lines.push(Line::from(""));

            // Syntax-highlighted content — indentation, no borders or boxes.
            // Tokenize the whole block, then split tokens on newlines so each
            // rendered line keeps its 8-space indent. Unknown languages yield a
            // single Plain token → same as the previous flat rendering.
            let trimmed = code.trim_end_matches('\n');
            let mut builder = LineBuilder::new();
            builder.push_raw(margin);
            builder.push("        ", theme.body()); // 8-space indent for code
            for tok in crate::highlight::highlight(trimmed, lang) {
                let style = theme.code_token(tok.kind);
                let mut parts = tok.text.split('\n');
                if let Some(first) = parts.next() {
                    builder.push(first, style);
                }
                for part in parts {
                    lines.push(builder.finish());
                    builder = LineBuilder::new();
                    builder.push_raw(margin);
                    builder.push("        ", theme.body());
                    builder.push(part, style);
                }
            }
            lines.push(builder.finish());

            lines.push(Line::from(""));
        }

        Element::Chart { chart } => {
            lines.push(Line::from(""));
            for row in chart_text_lines(chart, width) {
                let mut builder = LineBuilder::new();
                builder.push_raw(margin);
                builder.push("    ", theme.body());
                builder.push(&row, theme.code_block());
                lines.push(builder.finish());
            }
            lines.push(Line::from(""));
        }

        Element::Mermaid { flowchart } => {
            lines.push(Line::from(""));
            for row in mermaid_text_lines(flowchart) {
                let mut builder = LineBuilder::new();
                builder.push_raw(margin);
                builder.push("    ", theme.body());
                builder.push(&row, theme.code_block());
                lines.push(builder.finish());
            }
            lines.push(Line::from(""));
        }

        Element::BlockQuote { elements, .. } => {
            lines.push(Line::from(""));

            for el in elements {
                let mut quote_lines: Vec<Line<'static>> = Vec::new();
                render_element_styled(
                    el,
                    &mut quote_lines,
                    "",
                    width.saturating_sub(BLOCKQUOTE_WIDTH_REDUCTION),
                    theme,
                );

                for line in quote_lines {
                    let is_empty = line.spans.iter().all(|s| s.content.trim().is_empty());

                    let mut builder = LineBuilder::new();
                    builder.push_raw(margin);

                    if is_empty {
                        builder.push("    ▎", theme.blockquote_border());
                    } else {
                        builder.push("    ▎ ", theme.blockquote_border());
                        // Preserve the original styled spans (bold, italic, etc.)
                        builder.spans.extend(line.spans);
                    }
                    lines.push(builder.finish());
                }
            }

            lines.push(Line::from(""));
        }

        Element::List {
            ordered,
            start,
            items,
            ..
        } => {
            for (i, item) in items.iter().enumerate() {
                let marker = styled_list_marker(*ordered, *start, i, item.task);

                let marker_width = marker.chars().count();
                let text_width = width.saturating_sub(marker_width);

                let wrapped_lines = wrap_spans_styled(&item.spans, text_width, theme);

                for (j, segments) in wrapped_lines.iter().enumerate() {
                    let mut builder = LineBuilder::new();
                    builder.push_raw(margin);

                    if j == 0 {
                        builder.push(&marker, theme.list_marker());
                    } else {
                        builder.push_raw(&" ".repeat(marker_width));
                    }

                    for segment in segments {
                        builder.push(&segment.text, segment.style);
                    }

                    lines.push(builder.finish());
                }

                for block in &item.blocks {
                    let nested_margin = format!("{}{}", margin, " ".repeat(NESTED_LIST_INDENT));
                    render_element_styled(
                        block,
                        lines,
                        &nested_margin,
                        width.saturating_sub(NESTED_LIST_INDENT),
                        theme,
                    );
                }
            }
            lines.push(Line::from(""));
        }

        Element::HorizontalRule { .. } => {
            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
            }

            let rule_margin = " ".repeat(margin.len() + width / 4);
            let mut builder = LineBuilder::new();
            builder.push_raw(&rule_margin);
            builder.push("─  ·  ─", theme.hr());
            lines.push(builder.finish());

            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
            }
        }

        Element::Table {
            headers,
            rows,
            aligns,
        } => {
            lines.push(Line::from(""));

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

            let natural_total: usize =
                natural_widths.iter().sum::<usize>() + (headers.len().max(1) - 1) * 3 + 4;

            if natural_total <= width {
                render_table_tabular(lines, margin, headers, rows, aligns, &natural_widths, theme);
            } else {
                render_table_cards(lines, margin, headers, rows, width, theme);
            }

            lines.push(Line::from(""));
        }
    }
}

/// Count the length of the next word in styled chars
fn count_next_word_len_styled(chars: &[StyledChar], start: usize) -> usize {
    chars[start..]
        .iter()
        .take_while(|c| !c.ch.is_whitespace())
        .count()
}

/// Group consecutive styled chars with the same style into segments
fn group_styled_chars_into_segments(chars: &[StyledChar]) -> Vec<StyledSegment> {
    if chars.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity(4);
    let mut current_text = String::with_capacity(chars.len());
    let mut current_style = chars[0].style;

    for sc in chars {
        if sc.style == current_style {
            current_text.push(sc.ch);
        } else {
            if !current_text.is_empty() {
                segments.push(StyledSegment {
                    text: current_text,
                    style: current_style,
                });
            }
            current_text = sc.ch.to_string();
            current_style = sc.style;
        }
    }

    if !current_text.is_empty() {
        segments.push(StyledSegment {
            text: current_text,
            style: current_style,
        });
    }

    segments
}

/// Word wrap spans while preserving inline styles.
fn wrap_spans_styled(spans: &[Span], width: usize, theme: &Theme) -> Vec<Vec<StyledSegment>> {
    if width == 0 {
        return vec![vec![StyledSegment {
            text: String::new(),
            style: Style::default(),
        }]];
    }

    let mut all_chars: Vec<StyledChar> = Vec::new();

    for span in spans {
        let (text, style) = match &span.kind {
            SpanKind::Text(t) => (t.clone(), theme.body()),
            SpanKind::Emphasis(t) => (t.clone(), theme.emphasis()),
            SpanKind::Strong(t) => (t.clone(), theme.strong()),
            SpanKind::StrongEmphasis(t) => (t.clone(), theme.strong_emphasis()),
            SpanKind::Code(t) => (t.clone(), theme.inline_code()),
            SpanKind::Image { alt, .. } => (format!("[image: {}]", alt), theme.emphasis()),
            SpanKind::Link { text, url } => {
                let link_text = format!("{} [→ {}]", text, url);
                (link_text, theme.link())
            }
            SpanKind::Strikethrough(t) => (t.clone(), theme.strikethrough()),
            SpanKind::FootnoteRef { number, .. } => {
                (crate::parser::superscript(*number), theme.body())
            }
            SpanKind::SoftBreak => (" ".to_string(), theme.body()),
            SpanKind::HardBreak => ("\n".to_string(), theme.body()),
        };

        for ch in text.chars() {
            all_chars.push(StyledChar { ch, style });
        }
    }

    let estimated_lines = (all_chars.len() / width.max(1)).max(1);
    let mut result: Vec<Vec<StyledSegment>> = Vec::with_capacity(estimated_lines);
    let mut current_line: Vec<StyledChar> = Vec::with_capacity(width);
    let mut current_len = 0;
    let mut i = 0;

    while i < all_chars.len() {
        let sc = &all_chars[i];

        if sc.ch == '\n' {
            result.push(group_styled_chars_into_segments(&current_line));
            current_line = Vec::with_capacity(width);
            current_len = 0;
            i += 1;
            continue;
        }

        if sc.ch.is_whitespace() {
            if current_len > 0 {
                let next_word_len = count_next_word_len_styled(&all_chars, i + 1);

                if current_len + 1 + next_word_len > width && next_word_len > 0 {
                    result.push(group_styled_chars_into_segments(&current_line));
                    current_line = Vec::with_capacity(width);
                    current_len = 0;
                    i += 1;
                    continue;
                }

                current_line.push(sc.clone());
                current_len += 1;
            }
            i += 1;
        } else {
            current_line.push(sc.clone());
            current_len += 1;
            i += 1;
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(group_styled_chars_into_segments(&current_line));
    }

    result
}

/// Render table in tabular (bordered) format.
fn render_table_tabular(
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    aligns: &[CellAlign],
    col_widths: &[usize],
    theme: &Theme,
) {
    let total_width: usize = col_widths.iter().sum::<usize>() + (headers.len().max(1) - 1) * 3 + 4;

    // Top border
    let mut builder = LineBuilder::new();
    builder.push_raw(margin);
    builder.push(
        &format!("  ┌{}┐", "─".repeat(total_width.saturating_sub(2))),
        theme.table_border(),
    );
    lines.push(builder.finish());

    // Header row
    let mut builder = LineBuilder::new();
    builder.push_raw(margin);
    builder.push("  │", theme.table_border());
    for (i, h) in headers.iter().enumerate() {
        let w = col_widths[i];
        let align = aligns.get(i).copied().unwrap_or(CellAlign::Left);
        builder.push(
            &format!(" {}", align_cell(h, w, align)),
            theme.table_header(),
        );
        if i < headers.len() - 1 {
            builder.push(" │", theme.table_border());
        }
    }
    builder.push(" │", theme.table_border());
    lines.push(builder.finish());

    // Separator
    let sep_parts: Vec<String> = col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
    let mut builder = LineBuilder::new();
    builder.push_raw(margin);
    builder.push(
        &format!("  ├{}┤", sep_parts.join("┼")),
        theme.table_border(),
    );
    lines.push(builder.finish());

    // Data rows
    for row in rows {
        let mut builder = LineBuilder::new();
        builder.push_raw(margin);
        builder.push("  │", theme.table_border());
        for i in 0..headers.len() {
            let c = row.get(i).map(String::as_str).unwrap_or("");
            let w = col_widths.get(i).copied().unwrap_or(10);
            let align = aligns.get(i).copied().unwrap_or(CellAlign::Left);
            builder.push(&format!(" {}", align_cell(c, w, align)), theme.body());
            if i < headers.len() - 1 {
                builder.push(" │", theme.table_border());
            }
        }
        builder.push(" │", theme.table_border());
        lines.push(builder.finish());
    }

    // Bottom border
    let mut builder = LineBuilder::new();
    builder.push_raw(margin);
    builder.push(
        &format!("  └{}┘", "─".repeat(total_width.saturating_sub(2))),
        theme.table_border(),
    );
    lines.push(builder.finish());
}

/// Render table as stacked cards (when too wide for tabular).
fn render_table_cards(
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    headers: &[String],
    rows: &[Vec<String>],
    width: usize,
    theme: &Theme,
) {
    let max_header_len = headers
        .iter()
        .map(|h| h.chars().count())
        .max()
        .unwrap_or(10);
    let label_width = max_header_len.min(TABLE_LABEL_MAX_WIDTH);
    let value_width = width.saturating_sub(label_width + BLOCKQUOTE_WIDTH_REDUCTION);

    for (row_idx, row) in rows.iter().enumerate() {
        if row_idx > 0 {
            lines.push(Line::from(""));
        }

        // Card header
        let separator_width = width.saturating_sub(8);
        let mut builder = LineBuilder::new();
        builder.push_raw(margin);
        builder.push(&format!("  ─── {} ", row_idx + 1), theme.table_border());
        builder.push(&"─".repeat(separator_width), theme.table_border());
        lines.push(builder.finish());

        // Fields
        for (i, header) in headers.iter().enumerate() {
            let value = row.get(i).map(|s| s.as_str()).unwrap_or("");
            let value_lines = wrap_text(value, value_width);

            for (line_idx, value_line) in value_lines.iter().enumerate() {
                let mut builder = LineBuilder::new();
                builder.push_raw(margin);
                if line_idx == 0 {
                    builder.push(
                        &format!("  {:<width$}", header, width = label_width),
                        theme.table_header(),
                    );
                    builder.push("  ", theme.body());
                } else {
                    builder.push_raw(&" ".repeat(label_width + 4));
                }
                builder.push(value_line, theme.body());
                lines.push(builder.finish());
            }
        }
    }
}

fn render_spans_to_string(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match &span.kind {
            SpanKind::Text(t) => t.clone(),
            SpanKind::Emphasis(t) => t.clone(),
            SpanKind::Strong(t) => t.clone(),
            SpanKind::StrongEmphasis(t) => t.clone(),
            SpanKind::Code(t) => format!("‹{}›", t),
            SpanKind::Image { alt, url } => format!("[image: {} ({})]", alt, url),
            SpanKind::Link { text, url } => format!("{} [→ {}]", text, url),
            SpanKind::Strikethrough(t) => t.clone(),
            SpanKind::FootnoteRef { number, .. } => crate::parser::superscript(*number),
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

fn styled_list_marker(
    ordered: bool,
    start: Option<u64>,
    index: usize,
    task: Option<bool>,
) -> String {
    match (ordered, task) {
        (true, Some(done)) => format!(
            "  {}. {} ",
            start.unwrap_or(1) + index as u64,
            if done { "☑" } else { "☐" }
        ),
        (true, None) => format!("  {}. ", start.unwrap_or(1) + index as u64),
        (false, Some(true)) => "  ☑  ".to_string(),
        (false, Some(false)) => "  ☐  ".to_string(),
        (false, None) => "  ◆  ".to_string(),
    }
}

fn plain_list_marker(
    ordered: bool,
    start: Option<u64>,
    index: usize,
    task: Option<bool>,
) -> String {
    match (ordered, task) {
        (true, Some(done)) => format!(
            "{}. [{}] ",
            start.unwrap_or(1) + index as u64,
            if done { "x" } else { " " }
        ),
        (true, None) => format!("{}. ", start.unwrap_or(1) + index as u64),
        (false, Some(done)) => format!("[{}] ", if done { "x" } else { " " }),
        (false, None) => "•  ".to_string(),
    }
}

fn align_cell(text: &str, width: usize, align: CellAlign) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    let pad = width - len;
    match align {
        CellAlign::Left => format!("{text}{}", " ".repeat(pad)),
        CellAlign::Right => format!("{}{text}", " ".repeat(pad)),
        CellAlign::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
    }
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
        Element::CodeBlock { code, .. } => {
            let mut output = String::from("\n");
            for line in code.lines() {
                output.push_str(&format!("{}        {}\n", margin, line));
            }
            output.push('\n');
            output
        }
        Element::Chart { chart } => {
            let mut output = String::from("\n");
            for row in chart_text_lines(chart, OPTIMAL_WIDTH) {
                output.push_str(&format!("{}    {}\n", margin, row));
            }
            output.push('\n');
            output
        }
        Element::Mermaid { flowchart } => {
            let mut output = String::from("\n");
            for row in mermaid_text_lines(flowchart) {
                output.push_str(&format!("{}    {}\n", margin, row));
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
        Element::List {
            ordered,
            start,
            items,
            ..
        } => {
            let mut output = String::new();
            for (i, item) in items.iter().enumerate() {
                let marker = plain_list_marker(*ordered, *start, i, item.task);
                let text = render_spans_to_string(&item.spans);
                output.push_str(&format!("{}  {}{}\n", margin, marker, text));

                for block in &item.blocks {
                    output.push_str(&render_element_plain(block, indent + 3));
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
        Element::Table {
            headers,
            rows,
            aligns,
        } => {
            let mut output = String::from("\n");

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

            let total_width: usize =
                col_widths.iter().sum::<usize>() + (headers.len().max(1) - 1) * 3 + 4;

            if total_width <= TABLE_CARD_THRESHOLD {
                output.push_str(&format!(
                    "{}┌{}┐\n",
                    margin,
                    "─".repeat(total_width.saturating_sub(2))
                ));

                let header_cells: Vec<String> = headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| {
                        let align = aligns.get(i).copied().unwrap_or(CellAlign::Left);
                        format!(" {} ", align_cell(h, col_widths[i], align))
                    })
                    .collect();
                output.push_str(&format!("{}│{}│\n", margin, header_cells.join("│")));

                let sep_parts: Vec<String> =
                    col_widths.iter().map(|w| "─".repeat(*w + 2)).collect();
                output.push_str(&format!("{}├{}┤\n", margin, sep_parts.join("┼")));

                for row in rows {
                    let row_cells: Vec<String> = (0..headers.len())
                        .map(|i| {
                            let c = row.get(i).map(String::as_str).unwrap_or("");
                            let w = col_widths.get(i).copied().unwrap_or(10);
                            let align = aligns.get(i).copied().unwrap_or(CellAlign::Left);
                            format!(" {} ", align_cell(c, w, align))
                        })
                        .collect();
                    output.push_str(&format!("{}│{}│\n", margin, row_cells.join("│")));
                }

                output.push_str(&format!(
                    "{}└{}┘\n",
                    margin,
                    "─".repeat(total_width.saturating_sub(2))
                ));
            } else {
                let max_header_len = headers
                    .iter()
                    .map(|h| h.chars().count())
                    .max()
                    .unwrap_or(10);
                let label_width = max_header_len.min(TABLE_LABEL_MAX_WIDTH);
                let separator_width = OPTIMAL_WIDTH.saturating_sub(8);

                for (row_idx, row) in rows.iter().enumerate() {
                    if row_idx > 0 {
                        output.push('\n');
                    }
                    output.push_str(&format!(
                        "{}─── {} {}\n",
                        margin,
                        row_idx + 1,
                        "─".repeat(separator_width)
                    ));

                    for (i, header) in headers.iter().enumerate() {
                        let value = row.get(i).map(|s| s.as_str()).unwrap_or("");
                        output.push_str(&format!(
                            "{}{:<width$}  {}\n",
                            margin,
                            header,
                            value,
                            width = label_width
                        ));
                    }
                }
            }

            output.push('\n');
            output
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CHART TEXT FALLBACK - charts as text for the --tui and --print readers
// ═══════════════════════════════════════════════════════════════════════════

/// Render a mermaid flowchart as a plain-text edge list — the non-graphical
/// fallback. Each edge becomes `From ──(label)──▶ To`; an edgeless graph just
/// lists its node labels.
fn mermaid_text_lines(fc: &crate::mermaid::Flowchart) -> Vec<String> {
    if fc.edges.is_empty() {
        return fc.nodes.iter().map(|n| n.label.clone()).collect();
    }
    fc.edges
        .iter()
        .map(|e| {
            let conn = match (&e.label, e.arrow) {
                (Some(l), true) => format!("──({l})──▶"),
                (Some(l), false) => format!("──({l})──"),
                (None, true) => "─────▶".to_string(),
                (None, false) => "──────".to_string(),
            };
            format!(
                "{}  {}  {}",
                fc.nodes[e.from].label, conn, fc.nodes[e.to].label
            )
        })
        .collect()
}

/// Render a chart as plain text rows: ranked block bars for bar/pie, a unicode
/// sparkline per series for line, a range summary for scatter. "Defined, not
/// pretty, not broken" — the fallback contract for non-graphical readers.
fn chart_text_lines(chart: &crate::chart::Chart, width: usize) -> Vec<String> {
    use crate::chart::{ChartKind, fmt_num};

    let mut out = Vec::new();
    if let Some(t) = &chart.title {
        out.push(t.clone());
    }
    if let Some(axis) = chart_axis_line(chart) {
        out.push(axis);
    }
    if !out.is_empty() {
        out.push(String::new());
    }
    let bar_w = width.clamp(16, 80) / 3; // bar column ≈ a third of the measure

    match chart.kind {
        ChartKind::Bar => {
            let max = chart.y_max();
            let label_w = chart
                .x
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
                .min(18);
            let single = chart.series.len() <= 1;
            for series in &chart.series {
                if !single {
                    out.push(format!("{}:", series.name));
                }
                let pad = if single { "" } else { "  " };
                for (i, &v) in series.y.iter().enumerate() {
                    let lbl = chart.x.get(i).cloned().unwrap_or_default();
                    out.push(format!("{pad}{}", bar_row(&lbl, v, max, label_w, bar_w)));
                }
            }
        }
        ChartKind::Pie => {
            let total: f64 = chart.data.iter().map(|(_, v)| *v).sum();
            let mut rows = chart.data.clone();
            rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let max = rows
                .iter()
                .map(|(_, v)| *v)
                .fold(0.0_f64, f64::max)
                .max(f64::MIN_POSITIVE);
            let label_w = rows
                .iter()
                .map(|(k, _)| k.chars().count())
                .max()
                .unwrap_or(0)
                .min(18);
            for (k, v) in rows {
                let bars = "█".repeat(((v / max) * bar_w as f64).round() as usize);
                let pct = if total > 0.0 { v / total * 100.0 } else { 0.0 };
                out.push(format!("{k:<label_w$} {bars} {} ({pct:.0}%)", fmt_num(v)));
            }
        }
        ChartKind::Line => {
            for series in &chart.series {
                let name = if series.name.is_empty() {
                    "series"
                } else {
                    &series.name
                };
                let (mn, mx) = minmax(&series.y);
                out.push(format!(
                    "{name}: {}  ({}..{})",
                    sparkline(&series.y),
                    fmt_num(mn),
                    fmt_num(mx)
                ));
            }
        }
        ChartKind::Scatter => {
            let xs: Vec<f64> = chart.points.iter().map(|p| p.0).collect();
            let ys: Vec<f64> = chart.points.iter().map(|p| p.1).collect();
            let (xmn, xmx) = minmax(&xs);
            let (ymn, ymx) = minmax(&ys);
            out.push(format!(
                "scatter: {} points  (x {}..{}, y {}..{})",
                chart.points.len(),
                fmt_num(xmn),
                fmt_num(xmx),
                fmt_num(ymn),
                fmt_num(ymx),
            ));
        }
    }
    out
}

fn chart_axis_line(chart: &crate::chart::Chart) -> Option<String> {
    match (&chart.xlabel, &chart.ylabel) {
        (Some(x), Some(y)) => Some(format!("x: {x} · y: {y}")),
        (Some(x), None) => Some(format!("x: {x}")),
        (None, Some(y)) => Some(format!("y: {y}")),
        (None, None) => None,
    }
}

/// One labeled horizontal bar: `label  ████ value`.
fn bar_row(label: &str, v: f64, max: f64, label_w: usize, bar_w: usize) -> String {
    let n = if max > 0.0 {
        ((v / max) * bar_w as f64).round() as usize
    } else {
        0
    };
    format!(
        "{label:<label_w$} {} {}",
        "█".repeat(n),
        crate::chart::fmt_num(v)
    )
}

/// A compact unicode sparkline of a numeric series.
fn sparkline(ys: &[f64]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let (mn, mx) = minmax(ys);
    let range = (mx - mn).max(f64::MIN_POSITIVE);
    ys.iter()
        .map(|&v| BLOCKS[(((v - mn) / range) * 7.0).round() as usize % 8])
        .collect()
}

/// Min and max of a slice, `(0, 0)` if empty.
fn minmax(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let mn = v.iter().copied().fold(f64::MAX, f64::min);
    let mx = v.iter().copied().fold(f64::MIN, f64::max);
    (mn, mx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chart, ChartKind};

    #[test]
    fn task_markers_survive_in_fallback_renderers() {
        assert_eq!(styled_list_marker(false, None, 0, Some(true)), "  ☑  ");
        assert_eq!(styled_list_marker(false, None, 0, Some(false)), "  ☐  ");
        assert_eq!(plain_list_marker(false, None, 0, Some(true)), "[x] ");
        assert_eq!(plain_list_marker(true, Some(3), 1, Some(false)), "4. [ ] ");
    }

    #[test]
    fn table_cell_alignment_is_explicit() {
        assert_eq!(align_cell("x", 5, CellAlign::Left), "x    ");
        assert_eq!(align_cell("x", 5, CellAlign::Right), "    x");
        assert_eq!(align_cell("x", 5, CellAlign::Center), "  x  ");
    }

    #[test]
    fn chart_axis_labels_are_available_to_text_fallbacks() {
        let chart = Chart {
            kind: ChartKind::Bar,
            title: None,
            xlabel: Some("Month".to_string()),
            ylabel: Some("Revenue".to_string()),
            x: vec!["Jan".to_string()],
            series: Vec::new(),
            points: Vec::new(),
            data: Vec::new(),
        };

        assert_eq!(
            chart_axis_line(&chart),
            Some("x: Month · y: Revenue".to_string())
        );
    }
}
