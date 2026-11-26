//! Raster tests for radial shadings.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::{PixmapSettings, RenderCache, RenderSettings, hayro_syntax::Pdf, render};

#[test]
fn extended_zero_radius_shading_is_transparent() {
    let pdf = Pdf::new(
        b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 4 0 R
/Resources << /Shading << /S 5 0 R >> >> >> endobj
4 0 obj << /Length 5 >> stream
/S sh
endstream endobj
5 0 obj << /ShadingType 3 /ColorSpace /DeviceGray /Coords [0 0.5 0 1 0.5 0]
/Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >> endobj
trailer << /Root 1 0 R >>
%%EOF"
            .to_vec(),
    )
    .unwrap();
    let pixmap = render(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings::default(),
    );
    assert_eq!(pixmap.data_as_u8_slice(), &[0; 4]);
}
