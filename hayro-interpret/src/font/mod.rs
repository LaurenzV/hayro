//! Interacting with the different kinds of PDF fonts.

use crate::context::Context;
use crate::context::InterpreterCache;
use crate::device::Device;
use crate::interpret::state::State;
use crate::util::hash128;
use crate::{CacheKey, InterpreterSettings, Paint};
use hayro_cmap::{BfString, WritingMode};
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, Rect};
use std::ops::Deref;

mod type3;

pub use hayro_font::{
    FallbackFontQuery, FontData, FontQuery, FontStretch, GlyphId, GlyphIdentifier, HintingCache,
    OutlineFontData, OutlineGlyph, StandardFont,
};
pub(crate) use hayro_font::{Font, FontCache, FontGlyph, UNITS_PER_EM};

/// A glyph that can be drawn.
pub enum Glyph<'a> {
    /// A glyph defined by an outline.
    Outline(OutlineGlyph),
    /// A type3 glyph, defined by PDF drawing instructions.
    Type3(Box<Type3Glyph<'a>>),
}

impl Glyph<'_> {
    /// Returns the Unicode code point for this glyph, if available.
    ///
    /// This method attempts to determine the Unicode character that this glyph
    /// represents. The exact fallback chain depends on the font type:
    ///
    /// **For Outline Fonts (Type1, TrueType, CFF):**
    /// 1. `ToUnicode` cmap
    /// 2. Glyph name → Unicode (via Adobe Glyph List)
    /// 3. Unicode naming conventions (e.g., "uni0041", "u0041")
    ///
    /// **For CID Fonts (Type0):**
    /// 1. `ToUnicode` cmap
    ///
    ///
    /// **For Type3 Fonts:**
    /// 1. `ToUnicode` cmap
    /// 2. Glyph name → Unicode (via Adobe Glyph List)
    /// 3. Unicode naming conventions (e.g., "uni0041", "u0041")
    ///
    /// Returns `None` if the Unicode value could not be determined.
    ///
    /// Please note that this method is still somewhat experimental and might
    /// not work reliably in all cases.
    pub fn as_unicode(&self) -> Option<BfString> {
        match self {
            Glyph::Outline(g) => g.as_unicode(),
            Glyph::Type3(g) => g.as_unicode(),
        }
    }

    /// Return the character code that selected this glyph.
    ///
    /// For composite fonts, this is the code read with the font's `CMap`, not the CID.
    pub fn char_code(&self) -> u32 {
        match self {
            Glyph::Outline(g) => g.char_code(),
            Glyph::Type3(g) => g.char_code(),
        }
    }
}

/// A glyph and its position within a [`GlyphRun`].
pub struct PositionedGlyph<'a> {
    pub(crate) glyph: Glyph<'a>,
    pub(crate) transform: Affine,
    pub(crate) adjustment: f32,
}

impl PositionedGlyph<'_> {
    /// Return the transform that places this glyph in text space.
    pub fn transform(&self) -> Affine {
        self.transform
    }

    /// Return the `TJ` adjustment that was applied directly before this glyph.
    ///
    /// The value is in thousandths of a unit of text space, as written in the
    /// `TJ` array, and is the sum of all numbers between the previous character
    /// and this one. A positive value moves the glyph to the left in horizontal
    /// writing mode and down in vertical writing mode. Glyphs that are not
    /// preceded by an adjustment return 0.
    pub fn adjustment(&self) -> f32 {
        self.adjustment
    }
}

impl<'a> Deref for PositionedGlyph<'a> {
    type Target = Glyph<'a>;

    fn deref(&self) -> &Self::Target {
        &self.glyph
    }
}

/// A fully materialized run of positioned glyphs.
pub struct GlyphRun<'run, 'a> {
    pub(crate) glyphs: &'run [PositionedGlyph<'a>],
    pub(crate) writing_mode: WritingMode,
    pub(crate) text_properties: TextProperties,
}

impl<'run, 'a> GlyphRun<'run, 'a> {
    /// Return the glyphs in the run.
    pub fn glyphs(&self) -> &'run [PositionedGlyph<'a>] {
        self.glyphs
    }

    /// Return the writing mode of the font the run was shown with.
    pub fn writing_mode(&self) -> WritingMode {
        self.writing_mode
    }

    /// Return the text state parameters the run was shown with.
    pub fn text_properties(&self) -> TextProperties {
        self.text_properties
    }
}

/// The text state parameters of a [`GlyphRun`].
///
/// These parameters are already applied to the transform of each glyph. They
/// are exposed for consumers like text extraction that need to know the
/// individual values, for example to tell a gap between words apart from the
/// spacing between characters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextProperties {
    /// The font size set with `Tf`.
    pub font_size: f32,
    /// The character spacing set with `Tc`, in unscaled text space units.
    pub char_spacing: f32,
    /// The word spacing set with `Tw`, in unscaled text space units.
    pub word_spacing: f32,
    /// The horizontal scaling set with `Tz`, as a factor where 1.0 means 100%.
    pub horizontal_scaling: f32,
    /// The text rise set with `Ts`, in unscaled text space units.
    pub rise: f32,
}

/// A type3 glyph.
#[derive(Clone)]
pub struct Type3Glyph<'a> {
    pub(crate) glyph: hayro_font::Type3Glyph<'a>,
    pub(crate) state: State<'a>,
    pub(crate) bbox: Rect,
    pub(crate) parent_resources: Resources<'a>,
    pub(crate) cache: InterpreterCache<'a>,
    pub(crate) xref: &'a XRef,
    pub(crate) settings: InterpreterSettings,
    pub(crate) nesting_depth: u32,
}

/// A glyph defined by PDF drawing instructions.
impl<'a> Type3Glyph<'a> {
    /// Draw the type3 glyph to the given device.
    pub fn interpret(
        &self,
        device: &mut impl Device<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
    ) {
        type3::render_glyph(self, transform, glyph_transform, paint, device);
    }

    /// Returns the Unicode code point for this glyph, if available.
    ///
    /// See [`Glyph::as_unicode`] for details on the fallback chain used.
    pub fn as_unicode(&self) -> Option<BfString> {
        self.glyph.as_unicode()
    }

    /// Get the original character code that selected this glyph.
    pub fn char_code(&self) -> u32 {
        self.glyph.char_code()
    }

    /// Get the advance width for this glyph.
    ///
    /// The advance width is how far to move horizontally after drawing
    /// this glyph before drawing the next one.
    pub fn advance_width(&self) -> Option<f32> {
        Some(self.glyph.advance_width())
    }
}

impl CacheKey for Type3Glyph<'_> {
    fn cache_key(&self) -> u128 {
        hash128(&(self.glyph.cache_key(), self.bbox.cache_key()))
    }
}

impl CacheKey for GlyphIdentifier {
    fn cache_key(&self) -> u128 {
        Self::cache_key(self)
    }
}

pub(crate) fn get_glyph<'a>(
    font: &Font<'a>,
    glyph: GlyphId,
    char_code: u32,
    ctx: &mut Context<'a>,
    resources: &Resources<'a>,
) -> Glyph<'a> {
    match font.glyph(glyph, char_code) {
        FontGlyph::Outline(outline) => Glyph::Outline(outline),
        FontGlyph::Type3(glyph) => {
            let nesting_depth = ctx.nesting_depth() + 1;

            Glyph::Type3(Box::new(Type3Glyph {
                glyph,
                state: ctx.snapshot(),
                bbox: ctx.bbox(),
                parent_resources: resources.clone(),
                cache: ctx.interpreter_cache.clone(),
                xref: ctx.xref,
                settings: ctx.settings.clone(),
                nesting_depth,
            }))
        }
    }
}
