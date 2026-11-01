use crate::glyph_simulator::GlyphSimulator;
use crate::true_type::{Width, read_encoding, read_widths};
use crate::util::CacheKey;
use crate::{CMapResolverFn, Encoding, UNITS_PER_EM, glyph_name_to_unicode, read_to_unicode};
use hayro_cmap::{BfString, CMap};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{CHAR_PROCS, FONT_MATRIX, RESOURCES};
use kurbo::Affine;
use rustc_hash::FxHashMap;
use skrifa::GlyphId;

/// A Type 3 font.
#[derive(Debug)]
pub struct Type3Font<'a> {
    widths: Vec<Width>,
    missing_width: f32,
    encoding: Encoding,
    encodings: FxHashMap<u8, String>,
    dict: Dict<'a>,
    char_procs: FxHashMap<String, Stream<'a>>,
    glyph_simulator: GlyphSimulator,
    matrix: Affine,
    to_unicode: Option<CMap>,
}

impl<'a> Type3Font<'a> {
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

        Some(Self {
            glyph_simulator: GlyphSimulator::new(),
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

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        self.code_to_ps_name(code)
            .map(|g| self.glyph_simulator.string_to_glyph(g))
            .unwrap_or(GlyphId::NOTDEF)
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        self.encodings
            .get(&code)
            .map(|s| s.as_str())
            .or_else(|| self.encoding.map_code(code))
    }

    /// Return the advance width of a character code, scaled to [`UNITS_PER_EM`].
    pub fn glyph_width(&self, code: u8) -> f32 {
        let w = match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => w,
            _ => self.missing_width,
        };
        (w * self.matrix.as_coeffs()[0] as f32) * UNITS_PER_EM
    }

    /// Return the Unicode value of a character code.
    pub fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
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

    /// Return the font matrix.
    pub fn matrix(&self) -> Affine {
        self.matrix
    }

    /// Return the content stream that draws a glyph.
    pub fn glyph_program(&self, glyph: GlyphId) -> Option<&Stream<'a>> {
        let name = self.glyph_simulator.glyph_to_string(glyph)?;
        self.char_procs.get(&name)
    }

    /// Return the resources of the font.
    pub fn resources(&self) -> Option<Dict<'a>> {
        self.dict.get(RESOURCES)
    }

    /// Return the cache key of the font.
    pub fn cache_key(&self) -> u128 {
        self.dict.cache_key()
    }
}
