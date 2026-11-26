//! Check axial and radial shading colors at device pixel centers.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
use kurbo::Affine;

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

#[test]
fn continuous_stitch_cusps() {
    for (pdf, t, bound, left, peak) in [
        (
            include_bytes!("gradients/cusp0.pdf").as_slice(),
            0.5 - 0.3934505358338356,
            0.10740133374929428,
            0.14117923,
            0.65686637,
        ),
        (
            include_bytes!("gradients/cusp1.pdf").as_slice(),
            0.5 / 10.22,
            0.04980468,
            0.0,
            61.51 / 255.0,
        ),
    ] {
        let expected = f64::round((left + (peak - left) * (t / bound)) * 255.0) as u8;
        let actual = raster(pdf, 1.0).sample(0, 0).r;
        assert!(
            actual.abs_diff(expected) <= 2,
            "t={t}, expected {expected}, got {actual}"
        );
    }
}

fn crop(data: &[u8], width: u16, height: u16, transform: Affine) -> Pixmap {
    let pdf = Pdf::new(data.to_vec()).unwrap();
    let mut ctx = RenderContext::new(width, height);
    hayro::render_into(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &mut ctx,
        transform,
    );
    ctx.flush();
    let mut image = Pixmap::new(width, height);
    ctx.render(&mut image, &mut Resources::default());
    image
}

#[test]
fn nearly_tangent_circles_cover_the_left_of_the_focus() {
    for (size, shift) in [(400, 0_u16), (160, 120)] {
        let transform =
            Affine::translate((-f64::from(shift), -f64::from(shift))) * Affine::scale(2.);
        let image = crop(
            include_bytes!("gradients/radial_tangent.pdf"),
            size,
            size,
            transform,
        );
        let pixel = image.sample(199 - shift, 199 - shift);
        assert!(pixel.r.abs_diff(26) <= 1 && pixel.a == 255, "{pixel:?}");
    }
}

#[test]
fn nested_focal_covers_the_left_edge() {
    let image = crop(
        include_bytes!("gradients/nested_focal.pdf"),
        1,
        1,
        Affine::IDENTITY,
    );
    assert_eq!(image.sample(0, 0).a, 255);
}

#[test]
fn concentric_annulus_matches_calculator() {
    let image = crop(
        include_bytes!("gradients/concentric_annulus.pdf"),
        600,
        2,
        Affine::IDENTITY,
    );
    for x in 0..600 {
        let reference = image.sample(x, 0);
        assert_eq!(reference.a, 255);
        assert_eq!(image.sample(x, 1), reference, "x={x}");
    }
}

#[test]
fn subpixel_axial_gradient_keeps_the_pixel_center_parameter() {
    let image = raster(include_bytes!("gradients/subpixel_axial.pdf"), 1.);
    assert_eq!(image.sample(1, 50).r, 128);
}

#[test]
fn nonextended_miter() {
    let image = raster(include_bytes!("gradients/nonextended_miter.pdf"), 1.);
    for (x, expected) in [(25, [255, 0, 0]), (32, [0, 0, 0])] {
        let p = image.sample(x, 50);
        assert_eq!([p.r, p.g, p.b], expected);
    }
}

#[test]
fn single_segment_shading_ignores_miter_limit() {
    let draw = |limit| {
        let ops = format!("/Pattern CS /P SCN 4 w {limit} M 200 253 m 219 257 l S");
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 512 512] /Resources << /Pattern << /P << /PatternType 2 /Shading << /ShadingType 2 /Coords [0 0 512 512] /ColorSpace /DeviceGray /Function 5 0 R >> >> >> >> /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length {} >>\nstream\n{ops}\nendstream\nendobj\n5 0 obj\n<< /FunctionType 4 /Domain [0 1] /Range [0 1] /Length 7 >>\nstream\n{{ abs }}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF",
            ops.len()
        );
        raster(pdf.as_bytes(), 1.)
    };
    let a = draw(10);
    let b = draw(100000);
    assert!(a.sample(210, 256).r < 200);
    assert_eq!(a.data_as_u8_slice(), b.data_as_u8_slice());
}
