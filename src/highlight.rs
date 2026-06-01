//! Lightweight, dependency-free syntax highlighting for fenced code blocks.
//!
//! This is intentionally *not* a full lexer per language — it's a single generic
//! tokenizer driven by a small [`LangSpec`] (comment markers, string quotes, and
//! keyword/type/builtin word lists). That keeps the whole thing a few hundred
//! lines with no dependencies, which matches mdview's lean-dependency ethos,
//! while still giving real, readable color to the languages people actually
//! paste into markdown.
//!
//! The tokenizer guarantees two things the renderers rely on:
//! 1. **Every byte of the input is covered exactly once** — concatenating all
//!    token texts reproduces the input. (So newlines/indentation are preserved
//!    and nothing is dropped.)
//! 2. **All token boundaries fall on UTF-8 char boundaries** — slicing is always
//!    valid even with non-ASCII content inside strings/comments.
//!
//! Unknown or empty languages return a single [`TokenKind::Plain`] token so the
//! caller renders the block exactly as before (no highlighting, no risk).

/// Semantic class of a token, mapped to a color by each renderer's theme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    /// Identifiers, whitespace, and anything unclassified — the base ink.
    Plain,
    /// Line or block comments.
    Comment,
    /// Language keywords (`fn`, `if`, `return`, …).
    Keyword,
    /// Primitive / common type names (`u32`, `String`, `bool`, …).
    Type,
    /// Literal constants (`true`, `false`, `null`, `None`, …).
    Builtin,
    /// String (and char) literals.
    Str,
    /// Numeric literals.
    Number,
    /// An identifier used as a function call (`name(`).
    Function,
    /// Operators and punctuation.
    Punct,
}

/// One highlighted slice of the source.
#[derive(Clone, Copy, Debug)]
pub struct Token<'a> {
    pub text: &'a str,
    pub kind: TokenKind,
}

/// Per-language tokenizer configuration.
struct LangSpec {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that open/close string literals (ASCII only).
    quotes: &'static [u8],
    /// Rust lifetimes and labels also start with `'`, but they are not strings.
    /// Keep this as a language-specific escape hatch instead of weakening
    /// single-quoted strings for Python/JS/etc.
    rust_lifetimes: bool,
    ignore_case: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    builtins: &'static [&'static str],
}

/// Tokenize `code` for `lang`. Returns one `Plain` token spanning the whole
/// input when the language is unknown/unsupported (so the caller renders it
/// unhighlighted).
pub fn highlight<'a>(code: &'a str, lang: &str) -> Vec<Token<'a>> {
    if code.is_empty() {
        return vec![Token {
            text: code,
            kind: TokenKind::Plain,
        }];
    }
    let Some(spec) = lang_spec(lang) else {
        return vec![Token {
            text: code,
            kind: TokenKind::Plain,
        }];
    };
    Tokenizer {
        code,
        b: code.as_bytes(),
        spec,
    }
    .run()
}

struct Tokenizer<'a> {
    code: &'a str,
    b: &'a [u8],
    spec: LangSpec,
}

impl<'a> Tokenizer<'a> {
    fn run(&self) -> Vec<Token<'a>> {
        let n = self.b.len();
        let mut out: Vec<Token<'a>> = Vec::new();
        let mut i = 0;

        while i < n {
            let rest = &self.code[i..];
            let c = self.b[i];

            // Whitespace (incl. newlines) — preserved as Plain.
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                let start = i;
                while i < n && matches!(self.b[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                push(&mut out, &self.code[start..i], TokenKind::Plain);
                continue;
            }

            // Line comment → to end of line.
            if let Some(p) = self
                .spec
                .line_comments
                .iter()
                .find(|p| rest.starts_with(**p))
            {
                let _ = p;
                let start = i;
                while i < n && self.b[i] != b'\n' {
                    i += self.clen(i);
                }
                push(&mut out, &self.code[start..i], TokenKind::Comment);
                continue;
            }

            // Block comment → to closing delimiter (or EOF).
            if let Some((open, close)) = self.spec.block_comment
                && rest.starts_with(open)
            {
                let start = i;
                i += open.len();
                while i < n && !self.code[i..].starts_with(close) {
                    i += self.clen(i);
                }
                i = (i + close.len()).min(n);
                push(&mut out, &self.code[start..i], TokenKind::Comment);
                continue;
            }

            // String / char literal.
            if self.spec.quotes.contains(&c) {
                if c == b'\'' && self.spec.rust_lifetimes && self.is_rust_lifetime_or_label(i) {
                    push(&mut out, &self.code[i..i + 1], TokenKind::Punct);
                    i += 1;
                    continue;
                }
                let quote = c;
                let multiline = quote == b'`';
                let start = i;
                i += 1;
                while i < n {
                    let cb = self.b[i];
                    if cb == b'\\' {
                        i += 1;
                        if i < n {
                            i += self.clen(i);
                        }
                        continue;
                    }
                    if cb == quote {
                        i += 1;
                        break;
                    }
                    if cb == b'\n' && !multiline {
                        break; // unterminated — don't swallow the rest of the file
                    }
                    i += self.clen(i);
                }
                push(&mut out, &self.code[start..i], TokenKind::Str);
                continue;
            }

            // Number literal.
            if c.is_ascii_digit() {
                let start = i;
                i += 1;
                while i < n {
                    let cb = self.b[i];
                    if cb.is_ascii_alphanumeric() || cb == b'.' || cb == b'_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                push(&mut out, &self.code[start..i], TokenKind::Number);
                continue;
            }

            // Identifier / keyword.
            if is_ident_start(c) {
                let start = i;
                i += 1;
                while i < n && is_ident_cont(self.b[i]) {
                    i += 1;
                }
                let word = &self.code[start..i];
                let lower;
                let w: &str = if self.spec.ignore_case {
                    lower = word.to_ascii_lowercase();
                    &lower
                } else {
                    word
                };
                let kind = if self.spec.keywords.contains(&w) {
                    TokenKind::Keyword
                } else if self.spec.types.contains(&w) {
                    TokenKind::Type
                } else if self.spec.builtins.contains(&w) {
                    TokenKind::Builtin
                } else if self.next_nonspace_is_paren(i) {
                    TokenKind::Function
                } else {
                    TokenKind::Plain
                };
                push(&mut out, word, kind);
                continue;
            }

            // Operators / punctuation — group a run of symbol bytes.
            if is_punct(c) {
                let start = i;
                while i < n && is_punct(self.b[i]) {
                    i += 1;
                }
                push(&mut out, &self.code[start..i], TokenKind::Punct);
                continue;
            }

            // Anything else (e.g. a lone non-ASCII glyph) → Plain, one char.
            let start = i;
            i += self.clen(i);
            push(&mut out, &self.code[start..i], TokenKind::Plain);
        }

        out
    }

    /// Byte length of the UTF-8 char starting at `i` (1..=4), clamped so we never
    /// step past the end.
    fn clen(&self, i: usize) -> usize {
        let byte = self.b[i];
        let len = if byte < 0x80 {
            1
        } else if byte >> 5 == 0b110 {
            2
        } else if byte >> 4 == 0b1110 {
            3
        } else if byte >> 3 == 0b1_1110 {
            4
        } else {
            1
        };
        len.min(self.b.len() - i).max(1)
    }

    /// After an identifier ending at `i`, is the next non-space character a `(`?
    /// (Used to color `name(` as a function call.)
    fn next_nonspace_is_paren(&self, i: usize) -> bool {
        self.code[i..].bytes().find(|b| !matches!(b, b' ' | b'\t')) == Some(b'(')
    }

    fn is_rust_lifetime_or_label(&self, quote: usize) -> bool {
        debug_assert_eq!(self.b[quote], b'\'');
        let Some(&next) = self.b.get(quote + 1) else {
            return false;
        };
        if !is_ident_start(next) {
            return false;
        }

        let mut i = quote + 2;
        while i < self.b.len() && is_ident_cont(self.b[i]) {
            i += 1;
        }

        // `'x'` is a char literal. `'a`, `'static`, and `'label:` are
        // lifetimes/labels and should not swallow the rest of the line.
        self.b.get(i) != Some(&b'\'')
    }
}

fn push<'a>(out: &mut Vec<Token<'a>>, text: &'a str, kind: TokenKind) {
    if !text.is_empty() {
        out.push(Token { text, kind });
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c == b'@'
}

fn is_ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn is_punct(c: u8) -> bool {
    matches!(
        c,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'<'
            | b'>'
            | b'!'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'?'
            | b':'
            | b';'
            | b','
            | b'.'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'@'
            | b'\\'
    )
}

/// Resolve a fence info-string language to a tokenizer spec. Case-insensitive;
/// returns `None` for unknown languages (→ rendered unhighlighted).
fn lang_spec(lang: &str) -> Option<LangSpec> {
    let l = lang.trim().to_ascii_lowercase();
    Some(match l.as_str() {
        "rust" | "rs" => LangSpec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            rust_lifetimes: true,
            ignore_case: false,
            keywords: &[
                "as",
                "async",
                "await",
                "break",
                "const",
                "continue",
                "crate",
                "dyn",
                "else",
                "enum",
                "extern",
                "fn",
                "for",
                "if",
                "impl",
                "in",
                "let",
                "loop",
                "match",
                "mod",
                "move",
                "mut",
                "pub",
                "ref",
                "return",
                "static",
                "struct",
                "super",
                "trait",
                "type",
                "unsafe",
                "use",
                "where",
                "while",
                "union",
                "macro_rules",
            ],
            types: &[
                "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16",
                "i32", "i64", "i128", "isize", "f32", "f64", "String", "Vec", "Option", "Result",
                "Box", "Rc", "Arc", "HashMap", "Self",
            ],
            builtins: &["true", "false", "None", "Some", "Ok", "Err", "self"],
        },
        "python" | "py" => LangSpec {
            line_comments: &["#"],
            block_comment: None,
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
                "try", "while", "with", "yield", "match", "case",
            ],
            types: &[
                "int", "float", "str", "bool", "bytes", "list", "dict", "set", "tuple", "object",
            ],
            builtins: &[
                "True", "False", "None", "self", "cls", "print", "len", "range",
            ],
        },
        "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" | "typescript" | "ts" | "tsx" => {
            LangSpec {
                line_comments: &["//"],
                block_comment: Some(("/*", "*/")),
                quotes: b"\"'`",
                rust_lifetimes: false,
                ignore_case: false,
                keywords: &[
                    "async",
                    "await",
                    "break",
                    "case",
                    "catch",
                    "class",
                    "const",
                    "continue",
                    "debugger",
                    "default",
                    "delete",
                    "do",
                    "else",
                    "export",
                    "extends",
                    "finally",
                    "for",
                    "function",
                    "if",
                    "import",
                    "in",
                    "instanceof",
                    "interface",
                    "let",
                    "new",
                    "of",
                    "return",
                    "static",
                    "super",
                    "switch",
                    "throw",
                    "try",
                    "type",
                    "typeof",
                    "var",
                    "void",
                    "while",
                    "yield",
                    "enum",
                    "implements",
                    "namespace",
                    "readonly",
                ],
                types: &[
                    "string", "number", "boolean", "any", "void", "never", "unknown", "object",
                    "symbol", "bigint", "Array", "Promise", "Record", "Map", "Set",
                ],
                builtins: &[
                    "true",
                    "false",
                    "null",
                    "undefined",
                    "this",
                    "NaN",
                    "Infinity",
                    "console",
                ],
            }
        }
        "go" | "golang" => LangSpec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'`",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "break",
                "case",
                "chan",
                "const",
                "continue",
                "default",
                "defer",
                "else",
                "fallthrough",
                "for",
                "func",
                "go",
                "goto",
                "if",
                "import",
                "interface",
                "map",
                "package",
                "range",
                "return",
                "select",
                "struct",
                "switch",
                "type",
                "var",
            ],
            types: &[
                "bool", "string", "int", "int8", "int16", "int32", "int64", "uint", "uint8",
                "uint16", "uint32", "uint64", "byte", "rune", "float32", "float64", "error", "any",
            ],
            builtins: &[
                "true", "false", "nil", "iota", "make", "len", "cap", "append",
            ],
        },
        "c" | "h" | "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hxx" => LangSpec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "auto",
                "break",
                "case",
                "class",
                "const",
                "constexpr",
                "continue",
                "default",
                "delete",
                "do",
                "else",
                "enum",
                "extern",
                "for",
                "friend",
                "goto",
                "if",
                "inline",
                "namespace",
                "new",
                "operator",
                "private",
                "protected",
                "public",
                "return",
                "sizeof",
                "static",
                "struct",
                "switch",
                "template",
                "this",
                "typedef",
                "typename",
                "union",
                "using",
                "virtual",
                "volatile",
                "while",
            ],
            types: &[
                "bool", "char", "double", "float", "int", "long", "short", "signed", "unsigned",
                "void", "size_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "wchar_t",
            ],
            builtins: &["true", "false", "nullptr", "NULL"],
        },
        "java" => LangSpec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "abstract",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "default",
                "do",
                "else",
                "enum",
                "extends",
                "final",
                "finally",
                "for",
                "if",
                "implements",
                "import",
                "instanceof",
                "interface",
                "new",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "static",
                "super",
                "switch",
                "synchronized",
                "this",
                "throw",
                "throws",
                "try",
                "void",
                "volatile",
                "while",
            ],
            types: &[
                "boolean", "byte", "char", "double", "float", "int", "long", "short", "String",
                "Integer", "Object", "List", "Map",
            ],
            builtins: &["true", "false", "null"],
        },
        "bash" | "sh" | "shell" | "zsh" | "console" => LangSpec {
            line_comments: &["#"],
            block_comment: None,
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case",
                "esac", "function", "in", "select", "return", "break", "continue", "local",
                "export", "source", "alias", "set", "unset",
            ],
            types: &[],
            builtins: &["true", "false", "echo", "cd", "exit", "read", "printf"],
        },
        "ruby" | "rb" => LangSpec {
            line_comments: &["#"],
            block_comment: None,
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[
                "begin",
                "break",
                "case",
                "class",
                "def",
                "do",
                "else",
                "elsif",
                "end",
                "ensure",
                "for",
                "if",
                "in",
                "module",
                "next",
                "redo",
                "rescue",
                "retry",
                "return",
                "then",
                "unless",
                "until",
                "when",
                "while",
                "yield",
                "require",
                "attr_accessor",
            ],
            types: &[],
            builtins: &["true", "false", "nil", "self", "puts", "print"],
        },
        "sql" => LangSpec {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: true,
            keywords: &[
                "select",
                "from",
                "where",
                "insert",
                "into",
                "values",
                "update",
                "set",
                "delete",
                "create",
                "table",
                "drop",
                "alter",
                "add",
                "index",
                "join",
                "left",
                "right",
                "inner",
                "outer",
                "on",
                "group",
                "by",
                "order",
                "having",
                "limit",
                "offset",
                "as",
                "and",
                "or",
                "not",
                "in",
                "is",
                "distinct",
                "union",
                "all",
                "primary",
                "key",
                "foreign",
                "references",
                "default",
            ],
            types: &[
                "int",
                "integer",
                "bigint",
                "varchar",
                "text",
                "boolean",
                "date",
                "timestamp",
                "numeric",
                "decimal",
                "serial",
                "uuid",
                "json",
                "jsonb",
            ],
            builtins: &["null", "true", "false"],
        },
        "json" | "jsonc" => LangSpec {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            quotes: b"\"",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[],
            types: &[],
            builtins: &["true", "false", "null"],
        },
        "yaml" | "yml" => LangSpec {
            line_comments: &["#"],
            block_comment: None,
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[],
            types: &[],
            builtins: &["true", "false", "null", "yes", "no", "~"],
        },
        "toml" => LangSpec {
            line_comments: &["#"],
            block_comment: None,
            quotes: b"\"'",
            rust_lifetimes: false,
            ignore_case: false,
            keywords: &[],
            types: &[],
            builtins: &["true", "false"],
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core invariant: tokens must reproduce the input byte-for-byte.
    fn assert_lossless(code: &str, lang: &str) {
        let toks = highlight(code, lang);
        let joined: String = toks.iter().map(|t| t.text).collect();
        assert_eq!(joined, code, "lossy tokenization for {lang}");
    }

    #[test]
    fn lossless_across_languages() {
        let rust = "fn main() {\n    // hi\n    let x = \"a\\\"b\";\n    foo(x, 42);\n}\n";
        assert_lossless(rust, "rust");
        assert_lossless("def f(x):\n    return x + 1  # add\n", "python");
        assert_lossless("const x = `tpl ${y}`; // c\n/* b */\n", "ts");
        assert_lossless("SELECT * FROM t WHERE a = 1; -- note\n", "sql");
        assert_lossless("{\n  \"k\": true,\n  \"n\": 1.5\n}\n", "json");
        // Non-ASCII inside a string must not split a char boundary.
        assert_lossless("let s = \"café — naïve\";\n", "rust");
    }

    #[test]
    fn unknown_language_is_single_plain_token() {
        let toks = highlight("anything at all\n", "brainfuck");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Plain);
    }

    #[test]
    fn classifies_common_tokens() {
        let toks = highlight("let n = 42; // c", "rust");
        let kind = |s: &str| toks.iter().find(|t| t.text == s).map(|t| t.kind);
        assert_eq!(kind("let"), Some(TokenKind::Keyword));
        assert_eq!(kind("42"), Some(TokenKind::Number));
        assert_eq!(kind("// c"), Some(TokenKind::Comment));
    }

    #[test]
    fn function_call_detected() {
        let toks = highlight("foo(1)", "rust");
        assert_eq!(toks[0].text, "foo");
        assert_eq!(toks[0].kind, TokenKind::Function);
    }

    #[test]
    fn rust_lifetimes_are_not_strings() {
        let toks = highlight(
            "fn f<'a>(x: &'a str) -> &'a str { x }\nlet c = 'x';",
            "rust",
        );
        assert_lossless(
            "fn f<'a>(x: &'a str) -> &'a str { x }\nlet c = 'x';",
            "rust",
        );
        assert!(
            toks.iter()
                .filter(|t| t.kind == TokenKind::Str)
                .all(|t| t.text == "'x'"),
            "only the char literal should be classified as a string: {toks:?}"
        );
    }

    #[test]
    fn sql_keywords_are_case_insensitive() {
        let toks = highlight("SELECT * FROM t", "sql");
        let kind = |s: &str| toks.iter().find(|t| t.text == s).map(|t| t.kind);
        // Uppercase keyword matches the lowercase word list, and the original
        // case is preserved in the emitted token text.
        assert_eq!(kind("SELECT"), Some(TokenKind::Keyword));
        assert_eq!(kind("FROM"), Some(TokenKind::Keyword));
        // But case-sensitive languages keep their case sensitivity.
        let rust = highlight("String string", "rust");
        assert_eq!(
            rust.iter().find(|t| t.text == "String").unwrap().kind,
            TokenKind::Type
        );
        assert_eq!(
            rust.iter().find(|t| t.text == "string").unwrap().kind,
            TokenKind::Plain
        );
    }
}
