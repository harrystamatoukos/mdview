# Kitchen Sink — mdview rich rendering reference

This file exercises every CommonMark + GFM construct and the edge cases that
usually break a renderer. Use it to triage aesthetics, element by element.

## Headings

# H1 The quick brown fox
## H2 The quick brown fox
### H3 The quick brown fox
#### H4 The quick brown fox
##### H5 The quick brown fox
###### H6 The quick brown fox

## Paragraphs and inline emphasis

This is a plain paragraph with a reasonable amount of text so we can judge line
height, measure (line length), and how soft wrapping looks across multiple lines
of body copy. Good typography lives or dies on the body paragraph.

Inline styles: *italic*, **bold**, ***bold italic***, `inline code`,
~~strikethrough~~, and a [hyperlink](https://example.com). Mixed: **bold with
`code` inside**, *italic with [a link](https://example.com) inside*, and a
`really_long_inline_code_token_that_might_overflow_the_measure_if_unhandled`.

A long autolink: <https://example.com/some/very/long/path?with=query&params=that&go=on>

## Blockquotes

> A single-level blockquote. It should be visually distinct — indent, rule, or
> tint — without shouting.
>
> > A nested blockquote inside the first. Nesting depth should read clearly.
> >
> > > Three levels deep.

> A blockquote containing **bold**, *italic*, `code`, and a [link](https://x.com),
> plus a list:
>
> - first
> - second

## Lists

Unordered:

- Apples
- Oranges
  - Navel
  - Blood
    - Deep nesting level three
- Pears

Ordered:

1. First
2. Second
   1. Nested a
   2. Nested b
3. Third

Loose list (blank lines between items — items become paragraphs):

- First item with its own paragraph of text that wraps onto a second line so we
  can see item spacing.

- Second item, also loose.

Mixed content in a list item:

1. A list item with a code block inside:

   ```rust
   fn main() {
       println!("nested code block");
   }
   ```

2. A list item with a blockquote:

   > quoted inside a list

Task list (GFM):

- [x] Completed task
- [ ] Incomplete task
- [ ] Task with **bold** and `code`

## Code blocks

Plain fenced block (no language):

```
plain text
  indented line
no syntax highlighting
```

Rust with a long line that may need horizontal handling:

```rust
fn longish(argument_one: u32, argument_two: u32, argument_three: u32) -> u32 {
    argument_one + argument_two + argument_three // a trailing comment here
}
```

Indented code block (4 spaces):

    let indented = true;
    // four-space indented code

## Tables

Simple:

| Name  | Role     | Years |
| ----- | -------- | ----- |
| Alice | Engineer | 5     |
| Bob   | Designer | 3     |

With alignment:

| Left | Center | Right |
| :--- | :----: | ----: |
| a    | b      | c     |
| longer cell | mid | 1000 |

Wide table (tests overflow / truncation / wrapping):

| Column one is fairly long | Column two also has content | Column three keeps going | Column four |
| --- | --- | --- | --- |
| value with a longer string of text | another longish value here | yet more text in this cell | end |

## Horizontal rule

Above the rule.

---

Below the rule.

## Images

![Alt text for an image](https://example.com/image.png)

Inline image in text: here is an ![icon](https://example.com/icon.png) mid-sentence.

## Inline edge cases

- Em dash — en dash – ellipsis … and "smart quotes" plus 'singles'.
- Unicode: café, naïve, 日本語, emoji 🎉 🚀 ✅, math ∑ ∫ π ≈.
- Escaped characters: \*not italic\*, \`not code\`, \# not a heading.
- HTML inline: <strong>bold via html</strong> and <em>em via html</em>.

## Footnotes (GFM)

Here is a statement with a footnote.[^1] And another.[^long]

[^1]: The footnote text.
[^long]: A longer footnote with **formatting** and a [link](https://example.com).

## Definition-ish / hard breaks

A line ending with two spaces forces a hard break  
and this is the next line, same paragraph.

End of kitchen sink.
