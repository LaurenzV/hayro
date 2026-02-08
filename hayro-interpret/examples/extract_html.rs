//! A small example that shows how you can extract unicode characters from
//! glyphs. Please note that Unicode extraction is still experimental, so it
//! might not work fully correctly in certain cases. Note also that generating
//! div elements for every single character is clearly not desirable and there
//! should be some word/sentence merging algorithm in-place, but this is
//! out-of-scope for this example.

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterSettings, Paint,
    PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

use std::fmt::Write;

use hayro_cmap::UnicodeString;
use kurbo::{Affine, BezPath, Point, Rect};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let relative_path = args
        .get(1)
        .expect("Please provide a relative path to the PDF file as the first argument");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let data = std::fs::read(path).unwrap();

    let pdf = Pdf::new(data).unwrap();

    let settings = InterpreterSettings::default();
    // Pass dummy values for bbox and initial transform, since we don't care about those.
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        pdf.xref(),
        settings,
    );

    // Run everything!
    let page = &pdf.pages()[0];

    let mut extractor = TextExtractor {
        dimensions: page.render_dimensions(),
        ..Default::default()
    };

    writeln!(extractor.text, "<meta charset='utf-8' /> ").unwrap();
    writeln!(extractor.text, "<!-- page {} -->", 0).unwrap();
    writeln!(
        extractor.text,
        "<div id='page{}' style='position: relative; width: {}px; height: {}px; border: 1px black solid'>",
        0,
        extractor.dimensions.0,
        extractor.dimensions.1
    ).unwrap();

    interpret_page(page, &mut context, &mut extractor);

    writeln!(extractor.text, "</div>").unwrap();

    print!("{}", extractor.text);
}

#[derive(Default)]
struct TextExtractor {
    text: String,
    dimensions: (f32, f32),
}

/// Implement `Device` for `TextExtractor`. We extract Unicode text from glyphs.
impl Device<'_> for TextExtractor {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}

    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'_>,
        transform: Affine,
        glyph_transform: Affine,
        _: &Paint<'_>,
        _: &GlyphDrawMode,
    ) {
        if let Some(unicode_char) = glyph.as_unicode() {
            // Apply vertical flip transformation to combined transform
            // to place origin at top-left corner.
            let flip_transform = Affine::translate((0.0, self.dimensions.1 as f64))
                * Affine::scale_non_uniform(1.0, -1.0);
            let transform = flip_transform * transform * glyph_transform;

            let point = Point::new(0.0, 0.0);
            let position = transform * point;

            writeln!(
                self.text,
                "<div style='position: absolute; color: black; left: {}px; top: {}px; font-size: {}pt'>{}</div>",
                position.x, position.y, 6, match unicode_char {
                    UnicodeString::Char(c) => c.to_string(),
                    UnicodeString::String(s) => s
                }
            ).unwrap();
        } else {
            // Fallback for glyphs without Unicode mapping.
            self.text.push('�');
        }
    }

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}

    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}

    fn set_blend_mode(&mut self, _: BlendMode) {}
}
