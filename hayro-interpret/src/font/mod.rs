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
use skrifa::GlyphId;
use std::ops::Deref;
use std::rc::Rc;

mod type3;

pub use hayro_font::{
    FallbackFontQuery, FontData, FontQuery, FontStretch, GlyphIdentifier, OutlineFontData,
    OutlineGlyph, StandardFont, Type3Font,
};
pub(crate) use hayro_font::{Font, UNITS_PER_EM};

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
}

impl PositionedGlyph<'_> {
    /// Return the transform that places this glyph in text space.
    pub fn transform(&self) -> Affine {
        self.transform
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
}

/// A type3 glyph.
#[derive(Clone)]
pub struct Type3Glyph<'a> {
    pub(crate) font: Rc<Type3Font<'a>>,
    pub(crate) glyph_id: GlyphId,
    pub(crate) state: State<'a>,
    pub(crate) bbox: Rect,
    pub(crate) parent_resources: Resources<'a>,
    pub(crate) cache: InterpreterCache<'a>,
    pub(crate) xref: &'a XRef,
    pub(crate) settings: InterpreterSettings,
    pub(crate) nesting_depth: u32,
    pub(crate) char_code: u32,
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
        self.font.char_code_to_unicode(self.char_code)
    }

    /// Get the original character code that selected this glyph.
    pub fn char_code(&self) -> u32 {
        self.char_code
    }

    /// Get the advance width for this glyph.
    ///
    /// The advance width is how far to move horizontally after drawing
    /// this glyph before drawing the next one.
    pub fn advance_width(&self) -> Option<f32> {
        u8::try_from(self.char_code)
            .ok()
            .map(|code| self.font.glyph_width(code))
    }
}

impl CacheKey for Type3Glyph<'_> {
    fn cache_key(&self) -> u128 {
        hash128(&(self.font.cache_key(), self.glyph_id, self.bbox.cache_key()))
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
) -> Option<Glyph<'a>> {
    if let Some(outline) = font.outline_glyph(glyph, char_code) {
        return Some(Glyph::Outline(outline));
    }

    let font = font.type3()?;
    let nesting_depth = ctx.nesting_depth() + 1;

    Some(Glyph::Type3(Box::new(Type3Glyph {
        font: font.clone(),
        glyph_id: glyph,
        state: ctx.snapshot(),
        bbox: ctx.bbox(),
        parent_resources: resources.clone(),
        cache: ctx.interpreter_cache.clone(),
        xref: ctx.xref,
        settings: ctx.settings.clone(),
        nesting_depth,
        char_code,
    })))
}
