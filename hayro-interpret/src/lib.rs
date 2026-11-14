/*!
A crate for interpreting PDF files.

This crate provides an abstraction to interpret the content of a PDF file and render them
into an abstract [`Device`], which clients can implement as needed. This allows you, for
example, to render PDF files to bitmaps (which is what the `hayro` crate does), or other formats
such as SVG.

It should be noted that this crate is still very much in development. Therefore it currently
lacks pretty much any documentation on how to use it. It's current API also only really makes it
useful for rendering to PNG or SVG, though this will be improved upon in the future.

# Examples
See the `examples` folder on the GitHub repository. Apart from that, you can also consult
the implementation of `hayro` and `hayro-svg` to get an idea on how to use this crate.

# Safety
This crate forbids unsafe code via a crate-level attribute.

# Cargo features
This crate has one optional feature:
- `embed-fonts`: PDF processors are required to support 14 predefined fonts that do not need to be
  embedded into a PDF file. If you enable this feature, hayro will embed a (permissively-licensed)
  substitute for each font, so that you don't have to implement your custom font loading logic. This
  will add around ~240KB to your binary.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod canvas;
mod context;
mod convert;
mod device;
mod function;
mod function_shading;
mod interpret;
mod mesh;
mod ocg;
mod types;
mod x_object;

pub mod color;
pub mod encode;
pub mod font;
pub mod gradient;
pub mod pattern;
pub mod shading;
pub mod util;

pub use cache::CacheKey;
pub use context::*;
pub use device::*;
pub use function::{Function, TransferFunction};
pub use hayro_cmap;
pub use hayro_syntax;
pub use interpret::*;
pub use kurbo;
pub use types::*;
pub use util::{RectExt, TransformExt};
pub use x_object::soft_mask::*;

#[cfg(test)]
mod tests {
    use crate::font::{Glyph, GlyphRun};
    use crate::{
        BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
    };
    use hayro_cmap::BfString;
    use hayro_syntax::Pdf;
    use kurbo::{Affine, BezPath, Rect};
    use std::sync::Arc;

    #[derive(Default)]
    struct RecordingDevice {
        text: String,
        x_positions: Vec<f64>,
    }

    impl<'a> Device<'a> for RecordingDevice {
        fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(
            &mut self,
            _: f32,
            _: Option<SoftMask<'a>>,
            _: BlendMode,
            _: bool,
            _: bool,
        ) {
        }
        fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
            for positioned in run.glyphs() {
                self.x_positions
                    .push(positioned.transform().translation().x);
                if let Glyph::Outline(outline) = &**positioned {
                    assert!(outline.outline().is_empty());
                }
                if let Some(BfString::Char(c)) = positioned.as_unicode() {
                    self.text.push(c);
                }
            }
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
        fn pop_clip(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    #[test]
    fn interpret_standard_font_without_font_resolver_data() {
        let pdf_bytes = b"%PDF-1.4\n\
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100]\n\
  /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>\n\
  /Contents 4 0 R >> endobj\n\
4 0 obj << /Length 31 >> stream\n\
BT /F1 10 Tf 10 50 Td (Hi) Tj ET\n\
endstream\nendobj\n\
trailer << /Root 1 0 R >>\n%%EOF";

        let pdf = Pdf::new(pdf_bytes.to_vec()).unwrap();
        let settings = InterpreterSettings {
            font_resolver: Arc::new(|_| None),
            cmap_resolver: Arc::new(|_| None),
            warning_sink: Arc::new(|_| {}),
            ..InterpreterSettings::default()
        };
        let cache = InterpreterCache::new();
        let mut ctx = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 200.0, 100.0),
            &cache,
            pdf.xref(),
            settings,
        );
        let mut device = RecordingDevice::default();
        interpret_page(&pdf.pages()[0], &mut ctx, &mut device);

        assert_eq!(device.text, "Hi");
        assert_eq!(device.x_positions.len(), 2);
        // Helvetica 'H' width is 722 upem -> 7.22 user units at 10pt starting at x = 10.
        assert!((device.x_positions[0] - 10.0).abs() < 1e-3);
        assert!((device.x_positions[1] - 17.22).abs() < 1e-2);
    }
}
