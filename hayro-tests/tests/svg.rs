use crate::{interpreter_settings, load_pdf, run_svg_test, svg_render_settings};
use hayro::hayro_interpret::kurbo::Affine;
use hayro::hayro_interpret::util::TransformExt;
use hayro_svg::RenderCache;
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

// TODO: Ideally those tests are also generated from the manifest files so they stay in sync.

#[test]
fn integration_coat_of_arms() {
    run_svg_test(
        "integration_coat_of_arms",
        "pdfs/custom/integration_coat_of_arms.pdf",
        None,
    );
}

#[test]
fn image_rgb8() {
    run_svg_test("image_rgb8", "pdfs/custom/image_rgb8.pdf", None);
}

#[test]
fn image_rgba8() {
    run_svg_test("image_rgba8", "pdfs/custom/image_rgba8.pdf", None);
}

#[test]
fn image_mask_alpha() {
    run_svg_test("image_mask_alpha", "pdfs/custom/image_mask_alpha.pdf", None);
}

#[test]
fn clip_path_evenodd() {
    run_svg_test(
        "clip_path_evenodd",
        "pdfs/custom/clip_path_evenodd.pdf",
        None,
    );
}

#[test]
fn clip_path_nested() {
    run_svg_test("clip_path_nested", "pdfs/custom/clip_path_nested.pdf", None);
}

#[test]
fn pdfbox_2814() {
    run_svg_test("pdfbox_2814", "downloads/pdfbox/2814.pdf", None);
}

#[test]
fn pdfbox_2726() {
    run_svg_test("pdfbox_2726", "downloads/pdfbox/2726.pdf", None);
}

#[test]
fn pdfbox_3640() {
    run_svg_test("pdfbox_3640", "downloads/pdfbox/3640.pdf", None);
}

#[test]
fn pdfbox_3647() {
    run_svg_test("pdfbox_3647", "downloads/pdfbox/3647.pdf", None);
}

#[test]
fn pdfbox_5795() {
    run_svg_test("pdfbox_5795", "downloads/pdfbox/5795.pdf", None);
}

#[test]
fn pdfbox_6024() {
    run_svg_test("pdfbox_6024", "downloads/pdfbox/6024.pdf", None);
}

#[test]
fn pdfbox_3000_3() {
    run_svg_test("pdfbox_3000_3", "downloads/pdfbox/3000_3.pdf", None);
}

#[test]
fn image_interpolate() {
    run_svg_test(
        "image_interpolate",
        "pdfs/custom/image_interpolate.pdf",
        None,
    );
}

#[test]
fn image_jbig2_4() {
    run_svg_test(
        "image_jbig2_4",
        "downloads/custom/image_jbig2_4.pdf",
        Some("..=0"),
    );
}

#[test]
fn text_rendering_stroking_clipping() {
    run_svg_test(
        "text_rendering_stroking_clipping",
        "pdfs/custom/text_rendering_stroking_clipping.pdf",
        None,
    );
}

#[test]
fn image_ccit_4() {
    run_svg_test("image_ccit_4", "pdfs/custom/image_ccit_4.pdf", None);
}

#[test]
fn gradient_on_rect() {
    run_svg_test("gradient_on_rect", "pdfs/custom/gradient_on_rect.pdf", None);
}

#[test]
fn gradient_on_rotated_rect() {
    run_svg_test(
        "gradient_on_rotated_rect",
        "pdfs/custom/gradient_on_rotated_rect.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_cache_key() {
    run_svg_test(
        "pattern_tiling_cache_key",
        "pdfs/custom/pattern_tiling_cache_key.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_simple() {
    run_svg_test(
        "pattern_tiling_simple",
        "pdfs/custom/pattern_tiling_simple.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_nested() {
    run_svg_test(
        "pattern_tiling_nested",
        "pdfs/custom/pattern_tiling_nested.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_rotated() {
    run_svg_test(
        "pattern_tiling_rotated",
        "pdfs/custom/pattern_tiling_rotated.pdf",
        None,
    );
}

#[test]
fn pdfium_41480161() {
    run_svg_test("pdfium_41480161", "downloads/pdfium/41480161.pdf", None);
}

#[test]
fn mask_luminance() {
    run_svg_test(
        "mask_luminance",
        "pdfs/custom/resvg_masking_mask_mask_type_luminance.pdf",
        None,
    );
}

#[test]
fn mask_alpha() {
    run_svg_test(
        "mask_alpha",
        "pdfs/custom/resvg_masking_mask_mask_type_alpha.pdf",
        None,
    );
}

#[test]
fn mask_with_clip_path() {
    run_svg_test(
        "mask_with_clip_path",
        "pdfs/custom/resvg_masking_mask_with_clip_path.pdf",
        None,
    );
}

#[test]
fn font_type3_shape_glyphs() {
    run_svg_test(
        "font_type3_shape_glyphs",
        "pdfs/custom/font_type3_shape_glyphs.pdf",
        None,
    );
}

#[test]
fn font_type3_filled_glyphs() {
    run_svg_test(
        "font_type3_filled_glyphs",
        "pdfs/custom/font_type3_filled_glyphs.pdf",
        None,
    );
}

#[test]
fn font_type3_stroked_glyphs() {
    run_svg_test(
        "font_type3_stroked_glyphs",
        "pdfs/custom/font_type3_stroked_glyphs.pdf",
        None,
    );
}

#[test]
fn pdfjs_issue13372() {
    run_svg_test("pdfjs_issue13372", "downloads/pdfjs/issue13372.pdf", None);
}

#[test]
fn pdfjs_issue2177() {
    run_svg_test("pdfjs_issue2177", "downloads/pdfjs/issue2177.pdf", None);
}

#[test]
fn pdfjs_issue8565() {
    run_svg_test("pdfjs_issue8565", "downloads/pdfjs/issue8565.pdf", None);
}

#[test]
fn fillrule_evenodd() {
    run_svg_test("fillrule_evenodd", "pdfs/custom/fillrule_evenodd.pdf", None);
}

#[test]
fn stroke_properties() {
    run_svg_test(
        "stroke_properties",
        "pdfs/custom/stroke_properties.pdf",
        None,
    );
}

#[test]
fn issue_isolate_shading_transform() {
    run_svg_test(
        "issue_isolate_shading_transform",
        "pdfs/custom/issue_isolate_shading_transform.pdf",
        None,
    );
}

#[test]
fn mask_bc() {
    run_svg_test("mask_bc", "pdfs/custom/mask_bc.pdf", None);
}

#[test]
fn mask_no_cs() {
    run_svg_test("mask_no_cs", "pdfs/custom/mask_no_cs.pdf", None);
}

#[test]
fn mask_luminosity_no_cs() {
    run_svg_test(
        "mask_luminosity_no_cs",
        "pdfs/custom/mask_luminosity_no_cs.pdf",
        None,
    );
}

#[test]
fn mask_reuse() {
    run_svg_test("mask_reuse", "pdfs/custom/mask_reuse.pdf", None);
}

#[test]
fn pdfjs_issue11279() {
    run_svg_test("pdfjs_issue11279", "downloads/pdfjs/issue11279.pdf", None);
}

#[test]
fn pdfjs_issue9650() {
    run_svg_test("pdfjs_issue9650", "downloads/pdfjs/issue9650.pdf", None);
}

#[test]
fn issue_typst_7269() {
    run_svg_test(
        "issue_typst_7269",
        "downloads/custom/issue_typst_7269.pdf",
        None,
    );
}

#[test]
fn issue_986() {
    run_svg_test("issue968", "pdfs/custom/issue968.pdf", None);
}

#[test]
fn fill_degenerate() {
    run_svg_test("fill_degenerate", "pdfs/custom/fill_degenerate.pdf", None);
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

#[test]
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
