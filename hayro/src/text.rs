//! Structured text extraction from PDF pages.
//!
//! # Design
//!
//! PDF text is rendered by a pipeline that decomposes strings into individual
//! glyphs:
//!
//! ```text
//! Content stream  →  Tj / TJ operators
//!                        ↓
//!                    show_text_string()   (per-character: read code → map glyph → draw)
//!                        ↓
//!                    Device::draw_glyph() (individual positioned glyphs)
//! ```
//!
//! By the time glyphs reach [`Device::draw_glyph`], the higher-level text
//! structure is lost: font size is buried in a transform matrix (scaled by
//! 1/UNITS_PER_EM), word boundaries are indistinguishable from kerning
//! adjustments, and line breaks are invisible position changes.
//!
//! This module recovers that structure by using [`Device::draw_text_span`],
//! which is called once per `Tj` string operand or `TJ` array element with
//! the original Unicode text, explicit font size, and per-glyph device-space
//! positions, before the information is decomposed.
//!
//! [`extract_text`] creates a lightweight [`TextExtractorDevice`] that
//! implements [`Device`] and captures only two things:
//!
//! 1. _Text spans_ via [`draw_text_span`](Device::draw_text_span): each
//!    span preserves PDF string boundaries (spaces within a string are real
//!    characters, not heuristic guesses).
//!
//! 2. _Marked-content tags_ via [`begin_marked_content`](Device::begin_marked_content)
//!    / [`end_marked_content`](Device::end_marked_content): tracking the
//!    tag stack lets us annotate each span with its innermost structure tag
//!    (e.g. `P`, `H1`) and detect
//!    block boundaries and artifact regions.
//!
//! All other drawing operations are no-ops.
//!
//! # Coordinate space
//!
//! Positions are in the page's device coordinate space with a _top-left
//! origin_ (the Y-flip from the page's initial transform), matching the
//! coordinate system used by [`render`](crate::render).  Font sizes in
//! [`TextSpan::font_size`] are the explicit `Tf` values; font sizes in
//! [`TextSpan::font_size_device`] account for the full CTM + text matrix.

use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::util::PageExt;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterSettings, Paint,
    PathDrawMode, SoftMask, TextSpan, interpret_page,
};
use kurbo::{Affine, BezPath, Rect};

/// Extract text spans from a page.
///
/// Returns a list of [`TextSpan`]s in the order they appear in the PDF content
/// stream. Each span corresponds to one `Tj` string operand or one string
/// element within a `TJ` array.
///
/// Positions are in the page's device coordinate space with a _top-left
/// origin_ (the same coordinate system used by [`render`](crate::render)).
///
/// # Example
///
/// ```no_run
/// use hayro::hayro_syntax::Pdf;
/// use hayro::hayro_interpret::InterpreterSettings;
/// use hayro::text::extract_text;
///
/// let data = std::fs::read("document.pdf").unwrap();
/// let pdf = Pdf::new(data).unwrap();
/// let page = &pdf.pages()[0];
/// let spans = extract_text(page, &InterpreterSettings::default());
///
/// for span in &spans {
///     println!("{}", span.text);
/// }
/// ```
pub fn extract_text(page: &Page<'_>, interpreter_settings: &InterpreterSettings) -> Vec<TextSpan> {
    let initial_transform = page.initial_transform(true);
    let (width, height) = page.render_dimensions();

    let mut ctx = Context::new(
        initial_transform,
        Rect::new(0.0, 0.0, width as f64, height as f64),
        page.xref(),
        interpreter_settings.clone(),
    );

    let mut device = TextExtractorDevice::new();
    interpret_page(page, &mut ctx, &mut device);
    device.spans
}

// ---------------------------------------------------------------------------
// Internal device that captures text spans and discards everything else.
// ---------------------------------------------------------------------------

struct TextExtractorDevice {
    spans: Vec<TextSpan>,
    /// Stack of marked-content tags (from BMC/BDC ... EMC).
    tag_stack: Vec<String>,
    /// Set to `true` when a block-level marked-content sequence begins;
    /// consumed (set back to `false`) by the next `draw_text_span` call.
    pending_block_start: bool,
    /// Nesting depth of `/Artifact` sequences.
    artifact_depth: usize,
}

impl TextExtractorDevice {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            tag_stack: Vec::new(),
            pending_block_start: false,
            artifact_depth: 0,
        }
    }
}

impl Device<'_> for TextExtractorDevice {
    // -- Text extraction (the only thing we care about) --------------------

    fn draw_text_span(&mut self, span: &TextSpan) {
        let mut span = span.clone();
        span.tag = self.tag_stack.last().cloned();
        span.is_block_start = self.pending_block_start;
        span.is_artifact = self.artifact_depth > 0;
        self.pending_block_start = false;
        self.spans.push(span);
    }

    // -- Marked content tracking (cheap, useful later) ---------------------

    fn begin_marked_content(&mut self, tag: &[u8], _mcid: Option<i32>) {
        let tag_str = String::from_utf8_lossy(tag).into_owned();
        if is_block_level_tag(&tag_str) {
            self.pending_block_start = true;
        }
        if tag_str == "Artifact" {
            self.artifact_depth += 1;
        }
        self.tag_stack.push(tag_str);
    }

    fn end_marked_content(&mut self) {
        if let Some(tag) = self.tag_stack.pop() {
            if tag == "Artifact" {
                self.artifact_depth = self.artifact_depth.saturating_sub(1);
            }
        }
    }

    // -- Required no-ops ---------------------------------------------------

    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<SoftMask<'_>>,
        _: BlendMode,
    ) {
    }

    fn draw_glyph(
        &mut self,
        _: &Glyph<'_>,
        _: Affine,
        _: Affine,
        _: &Paint<'_>,
        _: &GlyphDrawMode,
    ) {
    }

    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}

    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn is_block_level_tag(tag: &str) -> bool {
    matches!(
        tag,
        "Document" | "Part" | "Art" | "Sect" | "Div"
            | "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6"
            | "P"
            | "L" | "LI" | "Lbl" | "LBody"
            | "Table" | "TR" | "TH" | "TD"
            | "BlockQuote" | "TOC" | "TOCI" | "Index"
            | "Figure" | "Formula"
            | "Artifact"
    )
}