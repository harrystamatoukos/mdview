use crate::parser::{self, Element, Span};
use crate::theme::{
    Theme, OPTIMAL_WIDTH, LEFT_MARGIN, MIN_MARGIN, TOP_PADDING,
    HEADING_SPACING_MAJOR, HEADING_SPACING_MINOR, SECTION_SPACING,
    BLOCKQUOTE_WIDTH_REDUCTION, NESTED_LIST_INDENT,
    TABLE_LABEL_MAX_WIDTH, TABLE_CARD_THRESHOLD,
};
use ratatui::text::{Line, Span as TuiSpan, Text};

/// Render markdown content to plain text (for --print mode)
pub fn render(content: &str) -> String {
    let elements = parser::parse(content);
    let mut output = String::new();

    for element in elements {
        output.push_str(&render_element_plain(&element, 0));
    }

    output
}

/// Render markdown to ratatui Text for TUI display
/// Centers content and applies book-like typography
pub fn render_to_text(content: &str, terminal_width: u16, theme: &Theme) -> Text<'static> {
    let elements = parser::parse(content);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Calculate centering: how much left margin to add
    let content_width = OPTIMAL_WIDTH.min(terminal_width as usize - LEFT_MARGIN);
    let left_margin = if terminal_width as usize > content_width + LEFT_MARGIN {
        (terminal_width as usize - content_width) / 2
    } else {
        MIN_MARGIN
    };
    let margin = " ".repeat(left_margin);

    // Top padding - like a book page
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }

    for element in elements {
        render_element_to_lines(&element, &mut lines, &margin, content_width, theme);
    }

    // End of document marker - subtle, centered
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }
    let end_marker_margin = " ".repeat(left_margin + content_width / 2 - 4);
    lines.push(Line::from(vec![
        TuiSpan::raw(end_marker_margin),
        TuiSpan::styled("· · ·", theme.hr()),
    ]));
    for _ in 0..TOP_PADDING {
        lines.push(Line::from(""));
    }

    Text::from(lines)
}

fn render_element_to_lines(
    element: &Element,
    lines: &mut Vec<Line<'static>>,
    margin: &str,
    width: usize,
    theme: &Theme,
) {
    match element {
        Element::Heading { level, text } => {
            // Major headers (H1/H2) get more spacing than minor headers
            let spacing_above = if *level <= 2 { HEADING_SPACING_MAJOR } else { HEADING_SPACING_MINOR };
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

            // H1 gets special treatment - more prominent
            if *level == 1 {
                lines.push(Line::from(vec![
                    TuiSpan::raw(margin.to_string()),
                    TuiSpan::styled(text.to_uppercase(), style),
                ]));
            } else {
                lines.push(Line::from(vec![
                    TuiSpan::raw(margin.to_string()),
                    TuiSpan::styled(text.clone(), style),
                ]));
            }

            // Subtle underline for H1 and H2
            if *level == 1 {
                lines.push(Line::from(""));
            } else if *level == 2 {
                // No underline, just spacing
            }

            lines.push(Line::from(""));
        }

        Element::Paragraph { spans } => {
            let text = render_spans_to_string(spans);

            // Word wrap to content width
            for wrapped_line in wrap_text(&text, width) {
                lines.push(Line::from(vec![
                    TuiSpan::raw(margin.to_string()),
                    TuiSpan::styled(wrapped_line, theme.body()),
                ]));
            }

            // Paragraph spacing
            lines.push(Line::from(""));
        }

        Element::CodeBlock { language, code } => {
            lines.push(Line::from(""));

            let lang_label = language.as_deref().unwrap_or("");

            // Top border with language
            lines.push(Line::from(vec![
                TuiSpan::raw(margin.to_string()),
                TuiSpan::styled(format!("    ╭─ {} ", lang_label), theme.code_border()),
                TuiSpan::styled("─".repeat(width.saturating_sub(10 + lang_label.len())), theme.code_border()),
            ]));

            // Code content with left border
            for code_line in code.lines() {
                lines.push(Line::from(vec![
                    TuiSpan::raw(margin.to_string()),
                    TuiSpan::styled("    │ ", theme.code_border()),
                    TuiSpan::styled(code_line.to_string(), theme.code_block()),
                ]));
            }

            // Bottom border
            lines.push(Line::from(vec![
                TuiSpan::raw(margin.to_string()),
                TuiSpan::styled(format!("    ╰{}─", "─".repeat(width.saturating_sub(8))), theme.code_border()),
            ]));

            lines.push(Line::from(""));
        }

        Element::BlockQuote { elements } => {
            lines.push(Line::from(""));

            // Render blockquote content with left bar
            for el in elements {
                let mut quote_lines: Vec<Line<'static>> = Vec::new();
                render_element_to_lines(el, &mut quote_lines, "", width.saturating_sub(BLOCKQUOTE_WIDTH_REDUCTION), theme);

                for line in quote_lines {
                    let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    if content.trim().is_empty() {
                        lines.push(Line::from(vec![
                            TuiSpan::raw(margin.to_string()),
                            TuiSpan::styled("    │", theme.blockquote_border()),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            TuiSpan::raw(margin.to_string()),
                            TuiSpan::styled("    │ ", theme.blockquote_border()),
                            TuiSpan::styled(content.trim().to_string(), theme.blockquote()),
                        ]));
                    }
                }
            }

            lines.push(Line::from(""));
        }

        Element::List { ordered, start, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("  {}. ", start.unwrap_or(1) + i as u64)
                } else {
                    "  •  ".to_string()
                };

                let text = render_spans_to_string(&item.spans);
                let marker_width = marker.chars().count();
                let text_width = width.saturating_sub(marker_width);

                let wrapped = wrap_text(&text, text_width);
                for (j, wrapped_line) in wrapped.iter().enumerate() {
                    if j == 0 {
                        lines.push(Line::from(vec![
                            TuiSpan::raw(margin.to_string()),
                            TuiSpan::styled(marker.clone(), theme.list_marker()),
                            TuiSpan::styled(wrapped_line.clone(), theme.body()),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            TuiSpan::raw(margin.to_string()),
                            TuiSpan::raw(" ".repeat(marker_width)),
                            TuiSpan::styled(wrapped_line.clone(), theme.body()),
                        ]));
                    }
                }

                // Handle nested lists
                if let Some(nested) = &item.nested {
                    let nested_margin = format!("{}{}", margin, " ".repeat(NESTED_LIST_INDENT));
                    render_element_to_lines(nested, lines, &nested_margin, width.saturating_sub(NESTED_LIST_INDENT), theme);
                }
            }
            lines.push(Line::from(""));
        }

        Element::HorizontalRule => {
            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
            }
            let rule_margin = " ".repeat(margin.len() + width / 4);
            lines.push(Line::from(vec![
                TuiSpan::raw(rule_margin),
                TuiSpan::styled("─  ·  ─", theme.hr()),
            ]));
            for _ in 0..SECTION_SPACING {
                lines.push(Line::from(""));
            }
        }

        Element::Table { headers, rows } => {
            lines.push(Line::from(""));

            // Calculate natural column widths (no truncation)
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

            // Calculate total natural width
            let natural_total: usize = natural_widths.iter().sum::<usize>() + (headers.len() - 1) * 3 + 4;

            // Decision: if table fits in width, use tabular; otherwise use card layout
            if natural_total <= width {
                // ═══════════════════════════════════════════════════════
                // TABULAR LAYOUT - table fits, render normally
                // ═══════════════════════════════════════════════════════
                render_table_tabular(lines, margin, headers, rows, &natural_widths, theme);
            } else {
                // ═══════════════════════════════════════════════════════
                // CARD LAYOUT - table too wide, render as cards
                // ═══════════════════════════════════════════════════════
                render_table_cards(lines, margin, headers, rows, width, theme);
            }

            lines.push(Line::from(""));
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
    spans
        .iter()
        .map(|span| match span {
            Span::Text(t) => t.clone(),
            Span::Emphasis(t) => t.clone(),
            Span::Strong(t) => t.clone(),
            Span::StrongEmphasis(t) => t.clone(),
            Span::Code(t) => format!("‹{}›", t),  // Subtle code markers
            Span::Link { text, url } => format!("{} [→ {}]", text, url),
            Span::Strikethrough(t) => t.clone(),
            Span::SoftBreak => " ".to_string(),
            Span::HardBreak => "\n".to_string(),
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
        Element::Heading { level, text } => {
            let spacing = if *level <= 2 {
                "\n".repeat(HEADING_SPACING_MAJOR)
            } else {
                "\n".repeat(HEADING_SPACING_MINOR)
            };
            if *level == 1 {
                format!("{}{}{}\n\n", spacing, margin, text.to_uppercase())
            } else {
                format!("{}{}{}\n\n", spacing, margin, text)
            }
        }
        Element::Paragraph { spans } => {
            let text = render_spans_to_string(spans);
            let wrapped: Vec<String> = wrap_text(&text, OPTIMAL_WIDTH)
                .iter()
                .map(|l| format!("{}{}", margin, l))
                .collect();
            format!("{}\n\n", wrapped.join("\n"))
        }
        Element::CodeBlock { language, code } => {
            let lang = language.as_deref().unwrap_or("");
            let border_width = OPTIMAL_WIDTH.saturating_sub(lang.len() + 5);
            let mut output = format!("\n{}╭─ {} {}\n", margin, lang, "─".repeat(border_width));
            for line in code.lines() {
                output.push_str(&format!("{}│ {}\n", margin, line));
            }
            output.push_str(&format!("{}╰{}\n\n", margin, "─".repeat(OPTIMAL_WIDTH.saturating_sub(3))));
            output
        }
        Element::BlockQuote { elements } => {
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
        Element::List { ordered, start, items } => {
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
        Element::HorizontalRule => {
            let hr_margin = " ".repeat(indent + OPTIMAL_WIDTH / 4);
            format!(
                "{}\n{}─  ·  ─\n{}\n",
                "\n".repeat(SECTION_SPACING),
                hr_margin,
                "\n".repeat(SECTION_SPACING)
            )
        }
        Element::Table { headers, rows } => {
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
