//! An invisible layer of the page's text, so that it can be selected, searched and copied
//! while the glyphs themselves stay paths.

use crate::SvgRenderer;
use hayro_interpret::font::{GlyphRun, PositionedGlyph};
use hayro_interpret::hayro_cmap::BfString;
use kurbo::{Affine, Point, Vec2};
use std::fmt::Write;

/// The advance assumed after the last glyph of a run, in glyph units. It places the
/// characters of a glyph that maps to several of them.
const LAST_GLYPH_ADVANCE: f64 = 500.0;

/// The text of a page, in the order in which it was drawn.
#[derive(Default)]
pub(crate) struct TextLayer {
    chunks: Vec<TextChunk>,
    /// The placement and character code of each glyph of the last recorded run.
    last_run: Vec<(Affine, u32)>,
}

/// Characters of a glyph run, written as one `text` element with the transform of its
/// first glyph.
struct TextChunk {
    transform: Affine,
    inverse: Affine,
    text: String,
    positions: Vec<Point>,
}

impl TextLayer {
    /// Record the text of a glyph run, whatever its draw mode. Glyphs without a Unicode
    /// mapping and glyphs drawn at zero size are left out.
    pub(crate) fn record(&mut self, glyph_run: &GlyphRun<'_, '_>, transform: Affine) {
        let glyphs = glyph_run.glyphs();
        // SVG draws text y-down, while glyph space is y-up.
        let place = |glyph: &PositionedGlyph<'_>| transform * glyph.transform() * Affine::FLIP_Y;

        // A run that draws the same glyphs at the same place as the previous one adds no
        // text. Text that is filled and stroked, for one, reaches the device as two runs.
        let run = glyphs.iter().map(|glyph| (place(glyph), glyph.char_code()));
        if run.clone().eq(self.last_run.iter().copied()) {
            return;
        }
        self.last_run.clear();
        self.last_run.extend(run);

        let first_chunk = self.chunks.len();

        for (index, glyph) in glyphs.iter().enumerate() {
            let Some(unicode) = glyph.as_unicode() else {
                continue;
            };
            let mut buf = [0; 4];
            let text = match &unicode {
                BfString::Char(c) => &*c.encode_utf8(&mut buf),
                BfString::String(s) => s.as_str(),
            };
            let chars = text.chars().filter_map(xml_char);
            let count = chars.clone().count();
            let placement = place(glyph);

            if count == 0 || !placement.is_finite() || !placement.determinant().is_normal() {
                continue;
            }

            let continues = self.chunks[first_chunk..]
                .last()
                .is_some_and(|chunk| chunk.accepts(placement));
            if !continues {
                self.chunks.push(TextChunk {
                    transform: placement,
                    inverse: placement.inverse(),
                    text: String::new(),
                    positions: Vec::new(),
                });
            }
            let chunk = self.chunks.last_mut().unwrap();

            let origin = chunk.position(placement);
            // The characters of a glyph that maps to several of them, like a ligature,
            // are spread up to the origin of the next glyph.
            let end = glyphs
                .get(index + 1)
                .filter(|_| count > 1)
                .map(|next| chunk.position(place(next)))
                .filter(|end| end.is_finite())
                .unwrap_or(origin + Vec2::new(LAST_GLYPH_ADVANCE, 0.0));

            for (at, c) in chars.enumerate() {
                chunk.text.push(c);
                chunk
                    .positions
                    .push(origin.lerp(end, at as f64 / count as f64));
            }
        }
    }
}

impl TextChunk {
    /// Whether a glyph placed with `placement` can join the chunk. Its horizontal scale may
    /// differ, since hayro stretches the glyphs of a substituted font to the widths in the
    /// PDF one by one.
    fn accepts(&self, placement: Affine) -> bool {
        let [a, b, c, d, _, _] = (self.inverse * placement).as_coeffs();

        a > 0.0 && b.abs() < 1e-3 && c.abs() < 1e-3 && (d - 1.0).abs() < 1e-3
    }

    /// Return the origin of a glyph placed with `placement`, relative to the chunk.
    fn position(&self, placement: Affine) -> Point {
        self.inverse * placement * Point::ORIGIN
    }
}

impl SvgRenderer<'_> {
    /// Write the text layer, above everything that the page painted.
    pub(crate) fn write_text_layer(&mut self, text_layer: TextLayer) {
        if text_layer.chunks.is_empty() {
            return;
        }

        self.xml.start_element("g");
        self.xml.write_attribute("id", "text-layer");
        self.xml.write_attribute("fill-opacity", "0");
        // Glyph space has 1000 units per em.
        self.xml.write_attribute("font-size", "1000");
        self.xml.write_attribute("xml:space", "preserve");

        for chunk in text_layer.chunks {
            self.xml.start_element("text");
            self.write_transform(chunk.transform);
            self.xml
                .write_attribute("x", &number_list(chunk.positions.iter().map(|p| p.x)));
            // Glyphs only leave the baseline in vertical text.
            if chunk.positions.iter().any(|p| p.y.abs() >= 0.05) {
                self.xml
                    .write_attribute("y", &number_list(chunk.positions.iter().map(|p| p.y)));
            }

            // Indentation would add whitespace to the text.
            self.xml.set_preserve_whitespaces(true);
            // `write_text` only escapes `<`.
            self.xml
                .write_text(&chunk.text.replace('&', "&amp;").replace('>', "&gt;"));
            self.xml.end_element();
            self.xml.set_preserve_whitespaces(false);
        }

        self.xml.end_element();
    }
}

/// Return the character that the layer writes for `c`. Tabs and line breaks become spaces,
/// as SVG displays them, and characters that XML doesn't allow are left out.
fn xml_char(c: char) -> Option<char> {
    match c {
        '\t' | '\n' | '\r' => Some(' '),
        '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'.. => Some(c),
        _ => None,
    }
}

/// Join numbers with spaces, rounded to a tenth of a glyph unit.
fn number_list(numbers: impl Iterator<Item = f64>) -> String {
    let mut list = String::new();

    for (index, number) in numbers.enumerate() {
        if index > 0 {
            list.push(' ');
        }

        // Adding zero turns `-0` into `0`.
        write!(list, "{}", ((number * 10.0).round() / 10.0 + 0.0) as f32).unwrap();
    }

    list
}
