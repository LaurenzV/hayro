//! Text span types for structured text extraction from PDF pages.
//!
//! Each [`TextSpan`] represents a contiguous run of text from a single `Tj`
//! string operand or one string element within a `TJ` array. Spans preserve
//! PDF string boundaries: spaces within a string are real characters, not
//! heuristic guesses.
//!
//! Glyph positions are in device space, with the page's initial transform
//! applied (Y-flip giving a top-left origin), matching the coordinate system
//! used by the renderer.


/// A span of text extracted from a PDF page.
///
/// Each span corresponds to one `Tj` string operand or one string element
/// within a `TJ` array.  Spans preserve PDF string boundaries: spaces within
/// a string are real characters, not guesses.
///
/// Glyph positions are in device space (with the Y-flip from the page's
/// initial transform applied, giving a top-left origin).  See
/// [`GlyphPosition`] for per-glyph details.
#[derive(Clone, Debug)]
pub struct TextSpan {
    /// The Unicode text content of this span.
    ///
    /// Assembled from per-glyph Unicode mappings (via `ToUnicode` `CMap`,
    /// glyph name fallback, etc.).  May be empty if the font lacks
    /// Unicode mappings.
    pub text: String,

    /// Per-glyph position and advance information.
    ///
    /// There is one entry per character code in the PDF string operand.
    /// Note that a single glyph may map to multiple Unicode characters
    /// (e.g. ligatures), so the lengths of [`glyphs`](Self::glyphs) and
    /// [`text`](Self::text) may differ.
    pub glyphs: Vec<GlyphPosition>,

    /// Font size in PDF points, as set by the `Tf` operator.
    ///
    /// This is the raw value from `TextState.font_size`, not
    /// reverse-engineered from transform matrices.
    pub font_size: f32,

    /// Font size in device-space units, accounting for the CTM and text matrix.
    ///
    /// Unlike [`font_size`](Self::font_size), which is the raw `Tf` value,
    /// this reflects the actual rendered height of the text. Use this for
    /// measuring text geometry (e.g. selection highlight rectangles).
    pub font_size_device: f32,

    /// The innermost marked-content tag active when this span was emitted
    /// (e.g. "H1", "P", "LBody"). `None` when no marked-content
    /// sequence is active.
    pub tag: Option<String>,

    /// `true` when a block-level marked-content sequence (heading,
    /// paragraph, list item, etc.) began before this span, i.e. this
    /// span is the first text inside a new structural element.
    pub is_block_start: bool,

    /// `true` when this span falls inside an `/Artifact` marked-content
    /// sequence (page headers, footers, page numbers, watermarks).
    pub is_artifact: bool,
}

/// Position and advance information for a single glyph.
#[derive(Clone, Debug)]
pub struct GlyphPosition {
    /// Unicode text for this glyph.
    ///
    /// Usually a single character, but may be multiple for ligatures
    /// (e.g. an "fi" ligature produces `"fi"`). Empty if the font lacks
    /// a Unicode mapping for this character code.
    pub text: String,

    /// X position in device space (top-left origin after Y-flip).
    pub x: f64,

    /// Y position in device space (top-left origin after Y-flip).
    pub y: f64,

    /// Advance width in the X direction in device space.
    ///
    /// For horizontal text this is the distance to the next glyph's
    /// origin. For vertical text this will be near zero.
    pub advance_x: f64,

    /// Advance width in the Y direction in device space.
    ///
    /// For vertical text this is the distance to the next glyph's
    /// origin. For horizontal text this will be near zero.
    pub advance_y: f64,

    /// The raw character code from the PDF content stream.
    ///
    /// This is the code used to look up the glyph in the font, before
    /// any Unicode mapping is applied.
    pub char_code: u32,
}