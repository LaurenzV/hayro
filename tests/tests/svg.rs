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
            Test::unit("svg::gradient_geometry", gradient_geometry),
            Test::unit("svg::steep_peak", steep_peak),
            Test::unit("svg::collapsed_band", collapsed_band),
            Test::unit("svg::close_stops", close_stops),
            Test::unit("svg::stitched_band", stitched_band),
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
