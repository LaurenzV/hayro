use crate::run_svg_test;

// TODO: Ideally those tests are also generated from the manifest files so they stay in sync.

#[test]
fn integration_coat_of_arms() {
    run_svg_test(
        "integration_coat_of_arms",
        "pdfs/integration_coat_of_arms.pdf",
        None,
    );
}

#[test]
fn image_rgb8() {
    run_svg_test("image_rgb8", "pdfs/image_rgb8.pdf", None);
}

#[test]
fn image_rgba8() {
    run_svg_test("image_rgba8", "pdfs/image_rgba8.pdf", None);
}

#[test]
fn clip_path_evenodd() {
    run_svg_test("clip_path_evenodd", "pdfs/clip_path_evenodd.pdf", None);
}

#[test]
fn clip_path_nested() {
    run_svg_test("clip_path_nested", "pdfs/clip_path_nested.pdf", None);
}

#[test]
fn pdfbox_2814() {
    run_svg_test("pdfbox_2814", "downloads/pdfbox/2814.pdf", None);
}

#[test]
fn image_interpolate() {
    run_svg_test("image_interpolate", "pdfs/image_interpolate.pdf", None);
}

#[test]
fn image_jbig2_4() {
    run_svg_test("image_jbig2_4", "downloads/image_jbig2_4.pdf", Some("..=0"));
}

#[test]
fn text_rendering_stroking_clipping() {
    run_svg_test(
        "text_rendering_stroking_clipping",
        "pdfs/text_rendering_stroking_clipping.pdf",
        None,
    );
}

#[test]
fn image_ccit_4() {
    run_svg_test("image_ccit_4", "pdfs/image_ccit_4.pdf", None);
}

#[test]
fn gradient_on_rect() {
    run_svg_test("gradient_on_rect", "pdfs/gradient_on_rect.pdf", None);
}

#[test]
fn gradient_on_rotated_rect() {
    run_svg_test(
        "gradient_on_rotated_rect",
        "pdfs/gradient_on_rotated_rect.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_simple() {
    run_svg_test(
        "pattern_tiling_simple",
        "pdfs/pattern_tiling_simple.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_nested() {
    run_svg_test(
        "pattern_tiling_nested",
        "pdfs/pattern_tiling_nested.pdf",
        None,
    );
}

#[test]
fn pattern_tiling_rotated() {
    run_svg_test(
        "pattern_tiling_rotated",
        "pdfs/pattern_tiling_rotated.pdf",
        None,
    );
}
