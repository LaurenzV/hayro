use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::manifest::entries;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{interpreter_settings, load_pdf, render_svg, svg_render_settings, try_load_pdf};
use hayro::hayro_interpret::kurbo::Affine;
use hayro::hayro_interpret::util::TransformExt;
use hayro_svg::RenderCache;
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

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
        .chain([Test::unit(
            "svg::convert_with_transform",
            convert_with_transform,
        )])
        .collect()
}

fn rasterize_svg(svg: &str, transform: Transform, size: (f32, f32)) -> Pixmap {
    let tree = Tree::from_data(svg.as_bytes(), &Options::default()).unwrap();
    let mut pixmap = Pixmap::new(size.0.ceil() as u32, size.1.ceil() as u32).unwrap();
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap
}

fn check_convert_with_transform(file_path: &str) {
    let pdf = load_pdf(file_path);
    let page = &pdf.pages()[0];
    let cache = RenderCache::new();
    let interpreter_settings = interpreter_settings();
    let render_settings = svg_render_settings();
    let (width, height) = page.render_dimensions();

    // Rotate by 90 degrees clockwise and scale by 2.
    let transform = Affine::new([0.0, 2.0, -2.0, 0.0, 2.0 * height as f64, 0.0]);
    let size = (2.0 * height, 2.0 * width);

    let expected = hayro_svg::convert(page, &cache, &interpreter_settings, &render_settings);
    let [a, b, c, d, e, f] = transform.as_coeffs().map(|c| c as f32);
    let expected = rasterize_svg(&expected, Transform::from_row(a, b, c, d, e, f), size);

    let actual = hayro_svg::convert_with_transform(
        page,
        &cache,
        &interpreter_settings,
        &render_settings,
        transform * page.initial_transform(true).to_kurbo(),
        size,
    );
    let actual = rasterize_svg(&actual, Transform::default(), size);

    // The coordinates written into the SVG are rounded differently, which
    // can change the anti-aliasing of a few edge pixels.
    let different = expected
        .pixels()
        .iter()
        .zip(actual.pixels())
        .filter(|(e, a)| e != a)
        .count();
    assert!(
        different <= expected.pixels().len() / 10_000,
        "{different} pixels differ for {file_path}"
    );
}

fn convert_with_transform() {
    for file_path in [
        "pdfs/custom/integration_coat_of_arms.pdf",
        "pdfs/custom/image_mask_alpha.pdf",
        "pdfs/custom/mask_bc.pdf",
        "pdfs/custom/clip_path_nested.pdf",
    ] {
        check_convert_with_transform(file_path);
    }
}
