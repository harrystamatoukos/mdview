use clap::ValueEnum;
use ratatui::style::{Color, Modifier, Style};

// Book-like reading theme based on typography research:
// - Line width: 60-70 characters (Bringhurst recommends 66)
// - Warm colors reduce eye strain
// - Generous whitespace is the #1 differentiator from code
// - High contrast (7:1+) but not harsh (avoid pure black/white)
// - Vertical rhythm: consistent spacing multiples
//
// See DESIGN_PRINCIPLES.md for the full design philosophy.

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

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum ThemeType {
    #[default]
    Paper,  // Warm, book-like (best for reading)
    Dark,   // Dark mode with warm undertones
    Light,  // Clean but not harsh
}

pub struct Theme {
    pub theme_type: ThemeType,
}

/// A complete set of colors for one theme variant.
///
/// All `Theme` style methods read from the palette chosen by `theme_type`,
/// so mdview paints its OWN page (background + foreground) rather than
/// borrowing the terminal's colors. This is what lets a light "paper" page
/// render correctly inside a dark terminal, and vice versa.
#[derive(Debug, Clone, Copy)]
struct Palette {
    /// Page background - painted across the whole screen
    canvas_bg: Color,
    /// Default body text ("ink")
    ink: Color,
    /// Heading colors, darkest/strongest (h1) to quietest (h6)
    h1: Color,
    h2: Color,
    h3: Color,
    h4: Color,
    h5: Color,
    h6: Color,
    /// Warm accent for bullets, bars, rules
    accent: Color,
    /// Inline + block code foreground
    code: Color,
    /// Link foreground
    link: Color,
    /// Subtle structural chrome (borders, grid lines)
    chrome: Color,
    /// Status bar foreground / background band
    status_fg: Color,
    status_bg: Color,
}

/// Warm "paper" page - light cream background, dark warm ink.
/// Used for both `Paper` and `Light`.
const LIGHT_PALETTE: Palette = Palette {
    canvas_bg: Color::Rgb(250, 244, 230), // #FAF4E6 warm linen
    ink: Color::Rgb(46, 42, 36),          // #2E2A24 warm near-black
    h1: Color::Rgb(138, 46, 18),          // #8A2E12 deep burnt rust
    h2: Color::Rgb(168, 69, 30),          // #A8451E rust
    h3: Color::Rgb(181, 101, 29),         // #B5651D ochre
    h4: Color::Rgb(156, 107, 63),         // #9C6B3F caramel
    h5: Color::Rgb(138, 109, 82),         // #8A6D52 muted warm brown
    h6: Color::Rgb(138, 109, 82),         // same as h5 (dimmed)
    accent: Color::Rgb(193, 91, 44),      // #C15B2C burnt orange
    code: Color::Rgb(150, 95, 60),        // #965F3C caramel
    link: Color::Rgb(46, 110, 100),       // #2E6E64 deep teal
    chrome: Color::Rgb(168, 156, 138),    // #A89C8A muted warm gray
    status_fg: Color::Rgb(110, 98, 83),   // #6E6253
    status_bg: Color::Rgb(237, 228, 208), // #EDE4D0 darker cream band
};

/// Warm dark page - charcoal background, warm light ink, brighter accents.
const DARK_PALETTE: Palette = Palette {
    canvas_bg: Color::Rgb(30, 27, 22),    // #1E1B16 warm charcoal
    ink: Color::Rgb(230, 220, 200),       // #E6DCC8 warm paper text
    h1: Color::Rgb(242, 166, 90),         // #F2A65A bright amber
    h2: Color::Rgb(232, 151, 90),         // #E8975A
    h3: Color::Rgb(217, 160, 102),        // #D9A066
    h4: Color::Rgb(201, 168, 126),        // #C9A87E
    h5: Color::Rgb(182, 168, 143),        // #B6A88F
    h6: Color::Rgb(182, 168, 143),        // same as h5 (dimmed)
    accent: Color::Rgb(232, 146, 74),     // #E8924A bright burnt orange
    code: Color::Rgb(224, 168, 106),      // #E0A86A
    link: Color::Rgb(131, 165, 152),      // #83A598 gruvbox blue-gray
    chrome: Color::Rgb(107, 95, 79),      // #6B5F4F
    status_fg: Color::Rgb(168, 155, 134), // #A89B86
    status_bg: Color::Rgb(42, 39, 31),    // #2A271F
};

#[allow(dead_code)] // API surface for future inline styling support
impl Theme {
    pub fn new(theme_type: ThemeType) -> Self {
        Self { theme_type }
    }

    /// The color set for the active theme variant.
    fn palette(&self) -> Palette {
        match self.theme_type {
            ThemeType::Paper | ThemeType::Light => LIGHT_PALETTE,
            ThemeType::Dark => DARK_PALETTE,
        }
    }

    // ─────────────────────────────────────────────────────────────
    // Canvas - the page itself. Painted across the whole screen so
    // mdview reads as a page regardless of the terminal's own colors.
    // ─────────────────────────────────────────────────────────────

    pub fn canvas(&self) -> Style {
        let p = self.palette();
        Style::default().fg(p.ink).bg(p.canvas_bg)
    }

    // ─────────────────────────────────────────────────────────────
    // Body text - explicit ink so it stays readable on the painted page
    // ─────────────────────────────────────────────────────────────

    pub fn body(&self) -> Style {
        Style::default().fg(self.palette().ink)
    }

    pub fn emphasis(&self) -> Style {
        self.body().add_modifier(Modifier::ITALIC)
    }

    pub fn strong(&self) -> Style {
        self.body().add_modifier(Modifier::BOLD)
    }

    pub fn strong_emphasis(&self) -> Style {
        self.body()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::ITALIC)
    }

    // ─────────────────────────────────────────────────────────────
    // Headers - warm colors with clear hierarchy steps, bold
    // ─────────────────────────────────────────────────────────────

    pub fn h1(&self) -> Style {
        Style::default().fg(self.palette().h1).add_modifier(Modifier::BOLD)
    }

    pub fn h2(&self) -> Style {
        Style::default().fg(self.palette().h2).add_modifier(Modifier::BOLD)
    }

    pub fn h3(&self) -> Style {
        Style::default().fg(self.palette().h3).add_modifier(Modifier::BOLD)
    }

    pub fn h4(&self) -> Style {
        Style::default().fg(self.palette().h4).add_modifier(Modifier::BOLD)
    }

    pub fn h5(&self) -> Style {
        Style::default().fg(self.palette().h5).add_modifier(Modifier::BOLD)
    }

    pub fn h6(&self) -> Style {
        // Same hue as H5 but dimmed - the quietest heading
        Style::default().fg(self.palette().h6).add_modifier(Modifier::DIM)
    }

    /// Left accent bar shown before H1 titles
    pub fn h1_accent(&self) -> Style {
        Style::default()
            .fg(self.accent_color())
            .add_modifier(Modifier::BOLD)
    }

    // ─────────────────────────────────────────────────────────────
    // Blockquotes - editorial feel
    // ─────────────────────────────────────────────────────────────

    pub fn blockquote(&self) -> Style {
        self.body().add_modifier(Modifier::ITALIC)
    }

    pub fn blockquote_border(&self) -> Style {
        // Warm accent bar - gives quotes a distinct, tinted edge
        Style::default()
            .fg(self.accent_color())
            .add_modifier(Modifier::BOLD)
    }

    // ─────────────────────────────────────────────────────────────
    // Code - a soft, legible warm tint that reads as code
    // ─────────────────────────────────────────────────────────────

    pub fn inline_code(&self) -> Style {
        Style::default().fg(self.palette().code)
    }

    pub fn code_block(&self) -> Style {
        Style::default().fg(self.palette().code)
    }

    pub fn code_border(&self) -> Style {
        Style::default()
            .fg(self.chrome_color())
            .add_modifier(Modifier::DIM)
    }

    // ─────────────────────────────────────────────────────────────
    // Links - recognizable but not screaming
    // ─────────────────────────────────────────────────────────────

    pub fn link(&self) -> Style {
        Style::default()
            .fg(self.palette().link)
            .add_modifier(Modifier::UNDERLINED)
    }

    // ─────────────────────────────────────────────────────────────
    // Lists - subtle accent on bullets
    // ─────────────────────────────────────────────────────────────

    pub fn list_marker(&self) -> Style {
        // Warm accent - bullets and numbers carry a touch of color
        Style::default()
            .fg(self.accent_color())
    }

    // ─────────────────────────────────────────────────────────────
    // Chrome - unified color for borders, rules, and decorative elements
    // ─────────────────────────────────────────────────────────────

    /// Base chrome color for all decorative/structural elements
    fn chrome_color(&self) -> Color {
        self.palette().chrome
    }

    /// Warm accent for structural punctuation: bullets, bars, rules
    fn accent_color(&self) -> Color {
        self.palette().accent
    }

    pub fn hr(&self) -> Style {
        // Warm accent rule - a clear but graceful section break
        Style::default()
            .fg(self.accent_color())
    }

    // ─────────────────────────────────────────────────────────────
    // Tables
    // ─────────────────────────────────────────────────────────────

    pub fn table_header(&self) -> Style {
        // Match h1
        Style::default().fg(self.palette().h1).add_modifier(Modifier::BOLD)
    }

    pub fn table_border(&self) -> Style {
        // Keep grid lines quiet so data stays the focus
        Style::default()
            .fg(self.chrome_color())
            .add_modifier(Modifier::DIM)
    }

    // ─────────────────────────────────────────────────────────────
    // Status bar - a quiet warm band, distinct from the page
    // ─────────────────────────────────────────────────────────────

    pub fn status_bar(&self) -> Style {
        let p = self.palette();
        Style::default().fg(p.status_fg).bg(p.status_bg)
    }

    pub fn status_bar_accent(&self) -> Style {
        let p = self.palette();
        Style::default().fg(p.accent).bg(p.status_bg)
    }

    // ─────────────────────────────────────────────────────────────
    // Cursor - clear insertion point indicator
    // ─────────────────────────────────────────────────────────────

    pub fn cursor(&self) -> Style {
        // Reversed colors make the cursor position unmistakable.
        // On the painted page this becomes an ink block on paper.
        Style::default()
            .add_modifier(Modifier::REVERSED)
    }

    pub fn strikethrough(&self) -> Style {
        Style::default()
            .fg(self.chrome_color())
            .add_modifier(Modifier::CROSSED_OUT)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeType::Paper)
    }
}
