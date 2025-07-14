use crate::run_write_test;
use sitro::Renderer;

#[test]
fn write_page_basic_1() {
    run_write_test(
        "write_page_basic_1",
        "pdfs/clip_path_evenodd.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_basic_2() {
    run_write_test(
        "write_page_basic_2",
        "pdfs/integration_coat_of_arms.pdf",
        &[0],
        Renderer::Mupdf,
    );
}

#[test]
fn write_page_basic_with_xobject() {
    run_write_test(
        "write_page_basic_with_xobject",
        "pdfs/xobject_1.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_basic_with_text() {
    run_write_test(
        "write_page_basic_with_text",
        "pdfs/pdftc_900k_0156_page_2.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_with_shading() {
    run_write_test(
        "write_page_shading",
        "downloads/pdfbox/1915_17.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_duplicated_page() {
    run_write_test(
        "write_page_duplicated_page",
        "pdfs/integration_diagram.pdf",
        &[0, 0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_mediabox_1() {
    run_write_test(
        "write_page_mediabox_1",
        "pdfs/page_media_box_bottom_left.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_mediabox_2() {
    run_write_test(
        "write_page_mediabox_2",
        "pdfs/page_media_box_top_left.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_mediabox_3() {
    run_write_test(
        "write_page_mediabox_3",
        "pdfs/page_media_box_zoomed_out.pdf",
        &[0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_multiple_pages_1() {
    run_write_test(
        "write_page_multiple_pages_1",
        "downloads/pdfbox/1772.pdf",
        &[0, 2, 1, 6, 8, 0],
        Renderer::Pdfium,
    );
}

#[test]
fn write_page_multiple_pages_2() {
    run_write_test(
        "write_page_multiple_pages_2",
        "downloads/pdfbox/2191.pdf",
        &[0, 1, 7],
        Renderer::Pdfium,
    );
}

// Original PDF contains reference for `ToUnicode`, but doesn't actually have it in the PDF.
#[test]
fn write_page_missing_ref() {
    run_write_test(
        "write_page_missing_ref",
        "downloads/pdfbox/5992_1.pdf",
        &[0],
        Renderer::Pdfium,
    );
}
