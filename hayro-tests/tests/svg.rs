use crate::run_svg_test;

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
fn pdfjs_issue11279() {
    run_svg_test("pdfjs_issue11279", "downloads/pdfjs/issue11279.pdf", None);
}
