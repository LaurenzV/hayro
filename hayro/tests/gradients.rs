//! Check axial and radial shading colors at device pixel centers.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::Pixmap;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

fn raster(data: &[u8], scale: f32) -> Pixmap {
    let pdf = Pdf::new(data.to_vec()).unwrap();
    render(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    )
}

#[test]
fn sampled_axial_peak() {
    let image = raster(include_bytes!("gradients/sampled_peak.pdf"), 1.0);
    for x in 0..100 {
        let gray = (1.0 - ((f64::from(x) + 0.5) / 12.5 - 3.0).abs()).max(0.0);
        assert_eq!(image.sample(x, 50).r, (gray * 255.0).round() as u8, "x={x}");
    }
}

#[test]
fn calculator_radial_rings() {
    let image = raster(include_bytes!("gradients/calculator_rings.pdf"), 2.0);
    for x in 0..200 {
        let radius = (f64::from(x) / 2.0 - 49.75).hypot(0.25);
        let gray = (radius / 100.0 * 46080.0).to_radians().sin().abs();
        assert_eq!(
            image.sample(x, 100).r,
            (gray * 255.0).round() as u8,
            "x={x}"
        );
    }
}

#[test]
fn nonextended_axial_endpoints() {
    let image = raster(include_bytes!("gradients/axial_ends.pdf"), 1.0);
    for (y, last_black) in [(0, 8), (1, 31), (2, 9)] {
        for x in 0..34 {
            let gray = if x <= last_black { 0 } else { 255 };
            assert_eq!(image.sample(x, y).r, gray, "x={x}, y={y}");
        }
    }
}

#[test]
fn diagonal_constant_calculator() {
    let short = raster(include_bytes!("gradients/diagonal_short.pdf"), 1.);
    let long = raster(include_bytes!("gradients/diagonal_long.pdf"), 1.);
    for y in 0..20 {
        for x in 0..40 {
            assert_eq!(short.sample(x, y), long.sample(x, y));
            assert_eq!(long.sample(x, y).r, 128);
        }
    }
}

#[test]
fn radial_clip_intersection_and_straight_stroke() {
    let image = raster(include_bytes!("gradients/clipped_radial.pdf"), 1.);
    for (y, operation) in [
        (174, "sh"),
        (124, "fill"),
        (74, "clipped stroke"),
        (24, "straight stroke"),
    ] {
        assert_eq!(image.sample(57, y).r, 255, "{operation}");
    }
}
