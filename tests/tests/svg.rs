use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::manifest::entries;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{interpreter_settings, render_svg, svg_render_settings, try_load_pdf};

pub(crate) fn tests() -> Vec<Test> {
    entries()
        .iter()
        .filter(|entry| entry.svg && !entry.ignore)
        .map(|entry| {
            let mut files = vec![entry.file.clone()];
            files.extend(FALLBACK_FONTS.map(String::from));

            Test::new(format!("svg::{}", entry.name), files, move || {
                let pdf = match try_load_pdf(&entry.file, &entry.password) {
                    Ok(pdf) => pdf,
                    Err(err) => return Outcome::Failed(err),
                };
                let pages = render_svg(
                    &pdf,
                    &entry.name,
                    interpreter_settings(),
                    svg_render_settings(),
                    entry.range.clone(),
                );
                check(Suite::Svg, &entry.name, pages, entry.max_diff_pixels)
            })
        })
        .chain([
            Test::unit("svg::shrinking_radial", || {
                radial_midpoint(
                    include_bytes!("../../hayro/tests/gradients/shrinking_radial.pdf"),
                    50,
                )
            }),
            Test::unit("svg::steep_annulus", || {
                radial_midpoint(
                    include_bytes!("../../hayro/tests/gradients/steep_annulus.pdf"),
                    0,
                )
            }),
            Test::unit("svg::gradient_cache_and_bbox", gradient_cache_and_bbox),
            Test::unit("svg::invariant_edges", invariant_edges),
            Test::unit("svg::gradient_geometry", gradient_geometry),
            Test::unit("svg::steep_peak", steep_peak),
            Test::unit("svg::gradient_miter_crop", miter_crop),
            Test::unit("svg::gradient_glyph_crop", glyph_crop),
            Test::unit("svg::gradient_offpage", offpage),
            Test::unit("svg::collapsed_band", collapsed_band),
            Test::unit("svg::close_stops", close_stops),
            Test::unit("svg::stitched_band", stitched_band),
            Test::unit("svg::gradient_fractional_edge", fractional_edge),
            Test::unit("svg::gradient_subpixel_mask", subpixel_mask),
            Test::unit("svg::gradient_narrow_encode", narrow_encode),
        ])
        .collect()
}

fn gradient(bytes: &[u8]) -> (String, resvg::tiny_skia::Pixmap) {
    scaled_gradient(bytes, 1)
}

fn scaled_gradient(bytes: &[u8], scale: u32) -> (String, resvg::tiny_skia::Pixmap) {
    let pdf = hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    let svg = hayro_svg::convert(
        &pdf.pages()[0],
        &Default::default(),
        &interpreter_settings(),
        &Default::default(),
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &Default::default()).unwrap();
    let size = tree.size().to_int_size();
    let mut image =
        resvg::tiny_skia::Pixmap::new(size.width() * scale, size.height() * scale).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
        &mut image.as_mut(),
    );
    (svg, image)
}

fn invariant_edges() {
    let data = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 8 8192] /Resources << /Shading << /S 5 0 R >> >> /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 5 >>\nstream\n/S sh\nendstream\nendobj\n5 0 obj\n<< /ShadingType 2 /Coords [0 0 8 0] /ColorSpace /DeviceGray /Function 6 0 R >>\nendobj\n6 0 obj\n<< /FunctionType 4 /Domain [0 1] /Range [0 1] /Length 7 >>\nstream\n{ abs }\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
    let (svg, image) = gradient(data);
    let doc = resvg::usvg::roxmltree::Document::parse(&svg).unwrap();
    let tex = doc.descendants().find(|n| n.has_tag_name("image")).unwrap();
    assert_eq!(tex.attribute("height"), Some("1"));
    for y in [0, 4096, 8191] {
        for x in 0..8 {
            let p = image.pixel(x, y).unwrap();
            assert_eq!(
                (p.red(), p.alpha()),
                ((((x as f32 + 0.5) * 255. / 8.).round() as u8), 255),
                "({x},{y})"
            );
        }
    }
}

fn collapsed_band() {
    let (svg, image) = gradient(include_bytes!(
        "../../hayro/tests/gradients/collapsed_band.pdf"
    ));
    assert!(svg.contains("<image"));
    for (x, gray) in [0, 255, 0].into_iter().enumerate() {
        let p = image.pixel(x as u32, 0).unwrap();
        assert_eq!(
            (p.red(), p.green(), p.blue(), p.alpha()),
            (gray, gray, gray, 255)
        );
    }
}

fn stitched_band() {
    let (svg, image) = scaled_gradient(
        include_bytes!("../../hayro/tests/gradients/stitched_band.pdf"),
        4,
    );
    assert!(svg.contains("<linearGradient"));
    let p = image.pixel(0, 8).unwrap();
    assert_eq!(
        (p.red(), p.green(), p.blue(), p.alpha()),
        (255, 255, 255, 255)
    );
    assert_eq!(image.pixel(1, 8).unwrap().red(), 0);
    assert_eq!(image.pixel(8, 8).unwrap().alpha(), 0);
}

fn miter_crop() {
    let (_, image) = scaled_gradient(
        include_bytes!("../../hayro/tests/gradients/miter_crop.pdf"),
        4,
    );
    assert!(image.pixel(26 * 4, 10 * 4).unwrap().alpha() > 0);
    assert_eq!(image.pixel(23 * 4, 10 * 4).unwrap().alpha(), 0);
}

fn glyph_crop() {
    let (_, image) = gradient(include_bytes!("../../hayro/tests/gradients/glyph_crop.pdf"));
    assert!((32..56).any(|x| (0..32).any(|y| image.pixel(x, y).unwrap().alpha() > 0)));
}

fn offpage() {
    let (svg, _) = gradient(include_bytes!(
        "../../hayro/tests/gradients/extended_exponential.pdf"
    ));
    assert!(svg.contains("<linearGradient"));
    assert!(!svg.contains("<image"));
    let (svg, image) = gradient(include_bytes!("../../hayro/tests/gradients/offpage.pdf"));
    let document = resvg::usvg::roxmltree::Document::parse(&svg).unwrap();
    let mut images = document
        .descendants()
        .filter(|node| node.has_tag_name("image"));
    let texture = images.next().unwrap();
    assert_eq!(texture.attribute("width"), Some("102"));
    assert_eq!(texture.attribute("height"), Some("1"));
    assert!(images.next().is_none());
    for y in 0..100 {
        for x in 0..100 {
            let expected = (((f64::from(x) + 0.5) / 100.0).powi(2) * 255.0).round() as u8;
            let pixel = image.pixel(x, y).unwrap();
            assert_eq!(pixel.alpha(), 255);
            assert!(
                pixel.red().abs_diff(expected) <= 1,
                "({x},{y}): {pixel:?} vs {expected}"
            );
        }
    }
}

fn fractional_edge() {
    let (_, image) = scaled_gradient(
        include_bytes!("../../hayro/tests/gradients/fractional_edge.pdf"),
        4,
    );
    let p = image.pixel(0, 8).unwrap();
    assert!((1..64).contains(&p.red()), "{p:?}");
    assert_eq!(p.alpha(), 255);
    assert_eq!(image.pixel(2, 8).unwrap().red(), 0);
}

fn subpixel_mask() {
    let (_, image) = gradient(include_bytes!(
        "../../hayro/tests/gradients/subpixel_mask.pdf"
    ));
    let p = image.pixel(0, 2).unwrap();
    assert!((1..255).contains(&p.alpha()), "{p:?}");
    assert_eq!(image.pixel(1, 2).unwrap().alpha(), 0);
}

fn narrow_encode() {
    for pdf in [
        include_bytes!("../../hayro/tests/gradients/narrow_type0.pdf").as_slice(),
        include_bytes!("../../hayro/tests/gradients/narrow_type3.pdf").as_slice(),
        include_bytes!("../../hayro/tests/gradients/nested_narrow.pdf").as_slice(),
    ] {
        let (_, image) = gradient(pdf);
        for x in 0..64 {
            assert_eq!(image.pixel(x, 0), image.pixel(x, 1), "x={x}");
        }
    }
}

fn close_stops() {
    for (pdf, band) in [
        (
            include_bytes!("../../hayro/tests/gradients/close_red.pdf").as_slice(),
            [255, 0, 0],
        ),
        (
            include_bytes!("../../hayro/tests/gradients/close_white.pdf").as_slice(),
            [255, 255, 255],
        ),
    ] {
        let (svg, image) = gradient(pdf);
        for x in 0..10 {
            let p = image.pixel(x, 0).unwrap();
            let expected = if (2..7).contains(&x) { band } else { [0; 3] };
            assert_eq!([p.red(), p.green(), p.blue()], expected, "x={x}");
            assert_eq!(p.alpha(), 255);
        }
        assert!(svg.contains("<image"));
    }
    for pdf in [
        include_bytes!("../../hayro/tests/gradients/close_padding.pdf").as_slice(),
        include_bytes!("../../hayro/tests/gradients/close_end_padding.pdf").as_slice(),
    ] {
        let (svg, image) = gradient(pdf);
        assert_eq!(image.pixel(3, 4).unwrap().alpha(), 0);
        assert_eq!(image.pixel(6, 4).unwrap().alpha(), 255);
        assert!(svg.contains("<image"));
    }
}

fn radial_midpoint(data: &[u8], x: u32) {
    let (svg, image) = gradient(data);
    let p = image.pixel(x, 0).unwrap();
    assert!(p.alpha() == 255 && p.red().abs_diff(128) <= 1, "{p:?}");
    assert!(svg.contains("<image"));
}

fn gradient_geometry() {
    let (svg, distinct) = gradient(include_bytes!(
        "../../hayro/tests/gradients/distinct_annulus.pdf"
    ));
    assert!(svg.contains("<image"));
    for (x, gray) in [4, 12, 20, 28].into_iter().enumerate() {
        assert_eq!(distinct.pixel(x as u32, 0).unwrap().red(), gray);
    }
    let (_, annulus) = gradient(include_bytes!(
        "../../hayro/tests/gradients/magnified_annulus.pdf"
    ));
    for (x, expected) in [32u8, 96, 159, 223].into_iter().enumerate() {
        let p = annulus.pixel(x as u32, 0).unwrap();
        assert!(p.alpha() == 255 && p.red().abs_diff(expected) <= 1, "{p:?}");
    }
    let (_, strip) = scaled_gradient(
        include_bytes!("../../hayro/tests/gradients/quarter_square.pdf"),
        8,
    );
    for x in 0..2 {
        let p = strip.pixel(x, 0).unwrap();
        assert_eq!(p.alpha(), 255, "{p:?}");
        assert!(p.red().abs_diff([16, 143][x as usize]) <= 3, "{p:?}");
    }
    for (data, expected) in [
        (
            include_bytes!("../../hayro/tests/gradients/reversed_boundary.pdf").as_slice(),
            &[255, 255, 0][..],
        ),
        (
            include_bytes!("../../hayro/tests/gradients/wide_boundary.pdf").as_slice(),
            &[0, 0, 0, 255, 255, 255][..],
        ),
    ] {
        let (_, image) = gradient(data);
        for (x, &gray) in expected.iter().enumerate() {
            assert_eq!(image.pixel(x as u32, 0).unwrap().red(), gray);
        }
    }
}

fn steep_peak() {
    let (svg, image) = gradient(include_bytes!("../../hayro/tests/gradients/steep_peak.pdf"));
    assert!(svg.contains("<image"));
    for x in 0..42 {
        let t = (f64::from(x) + 0.5) / 20.;
        let expected = (255. * (1. - (t - 1.).abs()).max(0.)).round() as u8;
        assert!(
            image.pixel(x, 0).unwrap().red().abs_diff(expected) <= 1,
            "x={x}"
        );
    }
}

fn gradient_cache_and_bbox() {
    for (pdf, x) in [
        (
            include_bytes!("../../hayro/tests/gradients/transfer_cache.pdf").as_slice(),
            3,
        ),
        (
            include_bytes!("../../hayro/tests/gradients/background_cache.pdf").as_slice(),
            0,
        ),
    ] {
        let (svg, image) = gradient(pdf);
        assert_eq!(svg.matches("<image ").count(), 2);
        let pixel = image.pixel(x, 2).unwrap();
        assert_eq!((pixel.red(), pixel.alpha()), (191, 255));
    }
    let (svg, image) = gradient(include_bytes!(
        "../../hayro/tests/gradients/bbox_visible.pdf"
    ));
    let doc = resvg::usvg::roxmltree::Document::parse(&svg).unwrap();
    let tex = doc.descendants().find(|n| n.has_tag_name("image")).unwrap();
    assert_eq!(
        (tex.attribute("width"), tex.attribute("height")),
        (Some("8"), Some("1"))
    );
    assert_eq!(image.pixel(12, 25).unwrap().alpha(), 255);
    let (svg, _) = gradient(include_bytes!(
        "../../hayro/tests/gradients/bbox_hidden.pdf"
    ));
    assert!(!svg.contains("<image"));
}
