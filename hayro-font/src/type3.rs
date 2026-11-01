use crate::GlyphId;
use crate::true_type::{Width, read_encoding, read_widths};
use crate::util::{CacheKey, hash128};
use crate::{CMapResolverFn, Encoding, UNITS_PER_EM, glyph_name_to_unicode, read_to_unicode};
use hayro_cmap::{BfString, CMap};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{CHAR_PROCS, FONT_MATRIX, RESOURCES};
use kurbo::Affine;
use rustc_hash::FxHashMap;
use std::rc::Rc;

/// A glyph of a Type 3 font, defined by a content stream of the font.
#[derive(Clone, Debug)]
pub struct Type3Glyph<'a> {
    font: Rc<Type3Font<'a>>,
    id: GlyphId,
    char_code: u32,
}

impl<'a> Type3Glyph<'a> {
    /// Return the content stream that draws the glyph.
    pub fn program(&self) -> Option<&Stream<'a>> {
        self.font.glyph_program(self.id)
    }

    /// Return the font of the glyph.
    pub fn font(&self) -> &Type3Font<'a> {
        &self.font
    }

    /// Returns the Unicode code point for this glyph, if available.
    pub fn as_unicode(&self) -> Option<BfString> {
        self.font.char_code_to_unicode(self.char_code)
    }

    /// Get the glyph ID within the font.
    pub fn glyph_id(&self) -> GlyphId {
        self.id
    }

    /// Get the original character code that selected this glyph.
    pub fn char_code(&self) -> u32 {
        self.char_code
    }

    /// Get the advance width for this glyph, scaled to [`UNITS_PER_EM`].
    pub fn advance_width(&self) -> f32 {
        self.font.glyph_width(self.char_code)
    }

    /// Return the cache key of the glyph.
    pub fn cache_key(&self) -> u128 {
        hash128(&(self.font.cache_key(), self.id))
    }

    pub(crate) fn new(font: Rc<Type3Font<'a>>, id: GlyphId, char_code: u32) -> Self {
        Self {
            font,
            id,
            char_code,
        }
    }
}

/// A Type 3 font.
#[derive(Debug)]
pub struct Type3Font<'a> {
    widths: Vec<Width>,
    missing_width: f32,
    encoding: Encoding,
    encodings: FxHashMap<u8, String>,
    dict: Dict<'a>,
    char_procs: FxHashMap<String, Stream<'a>>,
    glyph_names: Vec<String>,
    matrix: Affine,
    to_unicode: Option<CMap>,
}

impl<'a> Type3Font<'a> {
    /// Return the font matrix.
    pub fn matrix(&self) -> Affine {
        self.matrix
    }

    /// Return the resources of the font.
    pub fn resources(&self) -> Option<Dict<'a>> {
        self.dict.get(RESOURCES)
    }

    pub(crate) fn new(dict: &Dict<'a>, cmap_resolver: &CMapResolverFn) -> Option<Self> {
        let (encoding, encodings) = read_encoding(dict);
        let (widths, missing_width) = read_widths(dict, dict)?;

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(FONT_MATRIX)
                .unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]),
        );

        let char_procs = {
            let mut procs = FxHashMap::default();
            let dict = dict.get::<Dict<'_>>(CHAR_PROCS).unwrap_or_default();

            for name in dict.keys() {
                if let Some(prog) = dict.get::<Stream<'_>>(name.as_ref()) {
                    procs.insert(name.as_str().to_string(), prog);
                }
            }

            procs
        };

        let to_unicode = read_to_unicode(dict, cmap_resolver);

        // Type 3 fonts don't have glyph IDs, so each glyph procedure is assigned one,
        // starting at 1 so that 0 remains `.notdef`.
        let mut glyph_names = char_procs.keys().cloned().collect::<Vec<_>>();
        glyph_names.sort_unstable();

        Some(Self {
            glyph_names,
            encoding,
            char_procs,
            widths,
            missing_width,
            encodings,
            matrix,
            dict: dict.clone(),
            to_unicode,
        })
    }

    pub(crate) fn glyph_program(&self, glyph: GlyphId) -> Option<&Stream<'a>> {
        let name = self
            .glyph_names
            .get((glyph.to_u32() as usize).checked_sub(1)?)?;
        self.char_procs.get(name)
    }

    pub(crate) fn glyph_width(&self, code: u32) -> f32 {
        let w = match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => w,
            _ => self.missing_width,
        };
        (w * self.matrix.as_coeffs()[0] as f32) * UNITS_PER_EM
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
        if let Some(to_unicode) = &self.to_unicode
            && let Some(c) = to_unicode.lookup_bf_string(char_code)
        {
            // Skip null character mappings and fall back to glyph name
            // lookup. Some PDFs have incorrect ToUnicode mappings that map
            // to U+0000.
            if c != BfString::Char('\0') {
                return Some(c);
            }
        }

        u8::try_from(char_code)
            .ok()
            .and_then(|code| self.code_to_ps_name(code))
            .and_then(|name| glyph_name_to_unicode(name, false))
    }

    pub(crate) fn cache_key(&self) -> u128 {
        self.dict.cache_key()
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        self.code_to_ps_name(code)
            .and_then(|name| {
                let index = self
                    .glyph_names
                    .binary_search_by(|n| n.as_str().cmp(name))
                    .ok()?;
                Some(GlyphId::new(index as u32 + 1))
            })
            .unwrap_or(GlyphId::NOTDEF)
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        self.encodings
            .get(&code)
            .map(|s| s.as_str())
            .or_else(|| self.encoding.map_code(code))
    }
}
