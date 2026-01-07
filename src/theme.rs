use ratatui::style::{Color, Modifier, Style};

/// Book-like reading theme based on typography research:
/// - Line width: 60-70 characters (Bringhurst recommends 66)
/// - Warm colors reduce eye strain
/// - Generous whitespace is the #1 differentiator from code
/// - High contrast (7:1+) but not harsh (avoid pure black/white)
/// - Vertical rhythm: consistent spacing multiples
///
/// See DESIGN_PRINCIPLES.md for the full design philosophy.

// ═══════════════════════════════════════════════════════════════════════════
// LAYOUT CONSTANTS
// All magic numbers extracted here for maintainability and consistency.
// ═══════════════════════════════════════════════════════════════════════════

/// Content width - the "Bringhurst number" (66 characters optimal)
pub const OPTIMAL_WIDTH: usize = 66;

/// Maximum content width for emergency fallback
#[allow(dead_code)] // Design system constant, reserved for future use
pub const MAX_WIDTH: usize = 76;

/// Left margin from terminal edge (also used for centering calculation)
pub const LEFT_MARGIN: usize = 4;

/// Minimum margin when terminal is too narrow
pub const MIN_MARGIN: usize = 2;

// ───────────────────────────────────────────────────────────────────────────
// Vertical Rhythm - all spacing in blank lines for consistent rhythm
// ───────────────────────────────────────────────────────────────────────────

/// Top padding before content starts (book-like page feel)
pub const TOP_PADDING: usize = 2;

/// Spacing after paragraphs
#[allow(dead_code)] // Design system constant, documents vertical rhythm
pub const PARAGRAPH_SPACING: usize = 1;

/// Spacing before major headers (H1, H2)
pub const HEADING_SPACING_MAJOR: usize = 3;

/// Spacing before minor headers (H3-H6)
pub const HEADING_SPACING_MINOR: usize = 2;

/// Spacing around section breaks (HR)
pub const SECTION_SPACING: usize = 2;

// ───────────────────────────────────────────────────────────────────────────
// Element Indentation
// ───────────────────────────────────────────────────────────────────────────

/// Indentation for blockquote content (space before │)
#[allow(dead_code)] // Design system constant, documents element spacing
pub const BLOCKQUOTE_INDENT: usize = 4;

/// Width reduction for blockquote content (indent + border + space)
pub const BLOCKQUOTE_WIDTH_REDUCTION: usize = 6;

/// Indentation for code blocks
#[allow(dead_code)] // Design system constant, documents element spacing
pub const CODE_INDENT: usize = 4;

/// Additional indentation for each nested list level
pub const NESTED_LIST_INDENT: usize = 5;

/// Width of list markers (e.g., "  •  " or "  1. ")
#[allow(dead_code)] // Design system constant, documents list typography
pub const LIST_MARKER_WIDTH: usize = 5;

// ───────────────────────────────────────────────────────────────────────────
// Table Layout
// ───────────────────────────────────────────────────────────────────────────

/// Width threshold for switching to card layout (when table is too wide)
pub const TABLE_CARD_THRESHOLD: usize = 70;

/// Maximum width for field labels in card layout
pub const TABLE_LABEL_MAX_WIDTH: usize = 15;

#[derive(Debug, Clone, Copy, Default)]
pub enum ThemeType {
    #[default]
    Paper,  // Warm, book-like (best for reading)
    Dark,   // Dark mode with warm undertones
    Light,  // Clean but not harsh
}

impl ThemeType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "dark" => ThemeType::Dark,
            "light" => ThemeType::Light,
            _ => ThemeType::Paper,
        }
    }
}

pub struct Theme {
    #[allow(dead_code)] // For future theme switching
    pub theme_type: ThemeType,
}

#[allow(dead_code)] // API surface for future inline styling support
impl Theme {
    pub fn new(theme_type: ThemeType) -> Self {
        Self { theme_type }
    }

    // ─────────────────────────────────────────────────────────────
    // Body text - let terminal handle it for best compatibility
    // Research: user's chosen terminal colors are usually optimal
    // ─────────────────────────────────────────────────────────────

    pub fn body(&self) -> Style {
        Style::default()
    }

    pub fn emphasis(&self) -> Style {
        Style::default().add_modifier(Modifier::ITALIC)
    }

    pub fn strong(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn strong_emphasis(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::ITALIC)
    }

    // ─────────────────────────────────────────────────────────────
    // Headers - warm accent colors, bold
    // Research: gold/amber tones are warm and readable
    // Headers need visual distinction but not jarring contrast
    // ─────────────────────────────────────────────────────────────

    pub fn h1(&self) -> Style {
        // Deep warm brown - like espresso, editorial elegance
        Style::default()
            .fg(Color::Rgb(92, 64, 51))     // #5C4033 - rich warm brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn h2(&self) -> Style {
        // Warm brown - slightly lighter
        Style::default()
            .fg(Color::Rgb(107, 83, 68))    // #6B5344 - warm brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn h3(&self) -> Style {
        // Muted warm brown
        Style::default()
            .fg(Color::Rgb(122, 99, 85))    // #7A6355 - softer brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn h4(&self) -> Style {
        // Even softer warm brown - still distinct but subtler
        Style::default()
            .fg(Color::Rgb(137, 114, 100))  // #897264 - muted warm brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn h5(&self) -> Style {
        // Very subtle warm tone
        Style::default()
            .fg(Color::Rgb(152, 129, 115))  // #988173 - light warm brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn h6(&self) -> Style {
        // Barely there but still warm
        Style::default()
            .fg(Color::Rgb(152, 129, 115))  // Same as H5 but dim
            .add_modifier(Modifier::DIM)
    }

    // ─────────────────────────────────────────────────────────────
    // Blockquotes - editorial feel
    // Research: indented, italic, subtle color
    // ─────────────────────────────────────────────────────────────

    pub fn blockquote(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::ITALIC)
    }

    pub fn blockquote_border(&self) -> Style {
        // Unified chrome color for structural elements
        Style::default()
            .fg(self.chrome_color())
    }

    // ─────────────────────────────────────────────────────────────
    // Code - subtle distinction, respects terminal colors
    // Per DESIGN_PRINCIPLES: code should recede, not dominate
    // ─────────────────────────────────────────────────────────────

    pub fn inline_code(&self) -> Style {
        // Subtle: just dim the text slightly, no background or color change
        // Reader's focus should be on content, not code styling
        Style::default()
            .add_modifier(Modifier::DIM)
    }

    pub fn code_block(&self) -> Style {
        // Use terminal default - code blocks are already visually separated
        // by indentation and whitespace
        Style::default()
    }

    pub fn code_border(&self) -> Style {
        // Very subtle - borders should almost disappear
        Style::default()
            .fg(self.chrome_color())
            .add_modifier(Modifier::DIM)
    }

    // ─────────────────────────────────────────────────────────────
    // Links - recognizable but not screaming
    // ─────────────────────────────────────────────────────────────

    pub fn link(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(131, 165, 152))  // Gruvbox blue-gray
            .add_modifier(Modifier::UNDERLINED)
    }

    // ─────────────────────────────────────────────────────────────
    // Lists - subtle accent on bullets
    // ─────────────────────────────────────────────────────────────

    pub fn list_marker(&self) -> Style {
        // Unified chrome color for structural elements
        Style::default()
            .fg(self.chrome_color())
    }

    // ─────────────────────────────────────────────────────────────
    // Chrome - unified color for borders, rules, and decorative elements
    // Consolidates: code_border, blockquote_border, hr, list_marker
    // ─────────────────────────────────────────────────────────────

    /// Base chrome color for all decorative/structural elements
    fn chrome_color(&self) -> Color {
        Color::Rgb(102, 92, 84)  // #665C54 - warm muted gray
    }

    pub fn hr(&self) -> Style {
        Style::default()
            .fg(self.chrome_color())
            .add_modifier(Modifier::DIM)  // Even more subtle for rules
    }

    // ─────────────────────────────────────────────────────────────
    // Tables
    // ─────────────────────────────────────────────────────────────

    pub fn table_header(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(92, 64, 51))     // Match h1 - warm brown
            .add_modifier(Modifier::BOLD)
    }

    pub fn table_border(&self) -> Style {
        self.hr()
    }

    // ─────────────────────────────────────────────────────────────
    // Status bar - minimal, doesn't distract
    // ─────────────────────────────────────────────────────────────

    pub fn status_bar(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::DIM)
    }

    pub fn status_bar_accent(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(152, 151, 26))   // Gruvbox green
    }

    pub fn strikethrough(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::CROSSED_OUT)
            .add_modifier(Modifier::DIM)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeType::Paper)
    }
}
