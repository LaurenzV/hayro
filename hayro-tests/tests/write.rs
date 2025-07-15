use crate::{load_pdf, run_write_test};
use pdf_writer::Ref;
use sitro::Renderer;

#[test]
fn write_page_basic_1() {
    run_write_test(
        "write_page_basic_1",
        "pdfs/clip_path_evenodd.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn dont_cache_page_references() {
    let hayro_pdf = load_pdf("pdfs/clip_path_evenodd.pdf");
    let extracted =
        hayro_write::extract_pages(&hayro_pdf, Ref::new(2), Ref::new(1), &[0, 0], true).unwrap();

    // Adobe Acrobat does not seem to like reusing the same page reference, so we must always
    // create a new one and not cache them.
    assert!(extracted.root_refs[0] != extracted.root_refs[1]);
}

#[test]
fn write_page_basic_2() {
    run_write_test(
        "write_page_basic_2",
        "pdfs/integration_coat_of_arms.pdf",
        &[0],
        Renderer::Mupdf,
        true,
    );
}

#[test]
fn write_page_basic_with_xobject() {
    run_write_test(
        "write_page_basic_with_xobject",
        "pdfs/xobject_1.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_basic_with_text() {
    run_write_test(
        "write_page_basic_with_text",
        "pdfs/pdftc_900k_0156_page_2.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_with_shading() {
    run_write_test(
        "write_page_shading",
        "downloads/pdfbox/1915_17.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_duplicated_page() {
    run_write_test(
        "write_page_duplicated_page",
        "pdfs/integration_diagram.pdf",
        &[0, 0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_mediabox_1() {
    run_write_test(
        "write_page_mediabox_1",
        "pdfs/page_media_box_bottom_left.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_rotation() {
    run_write_test(
        "write_page_rotation",
        "pdfs/page_rotation_270.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_multiple_pages_1() {
    run_write_test(
        "write_page_multiple_pages_1",
        "downloads/pdfbox/1772.pdf",
        &[0, 2, 1, 6, 8, 0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_multiple_pages_2() {
    run_write_test(
        "write_page_multiple_pages_2",
        "downloads/pdfbox/2191.pdf",
        &[0, 1, 7],
        Renderer::Pdfium,
        true,
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
        true,
    );
}

#[test]
fn write_page_with_inherited_resources_1() {
    run_write_test(
        "write_page_with_inherited_resource",
        "downloads/pdfbox/5910.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_with_inherited_resources_2() {
    run_write_test(
        "write_page_with_inherited_resources_2",
        "pdfs/pdfjs/issue17065.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

// Not writing the `Properties` entry of `Resources` causes rendering issues in
// Quartz, and ghostscript prints a warning.
#[cfg(target_os = "macos")]
#[ignore]
#[test]
fn write_page_with_properties() {
    run_write_test(
        "write_page_with_properties",
        "downloads/pdfbox/3754.pdf",
        &[0],
        Renderer::Quartz,
        true,
    );
}

#[test]
fn write_xobject_basic_1() {
    run_write_test(
        "write_xobject_basic_1",
        "pdfs/clip_path_evenodd.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_basic_2() {
    run_write_test(
        "write_xobject_basic_2",
        "pdfs/integration_coat_of_arms.pdf",
        &[0],
        Renderer::Mupdf,
        false,
    );
}

#[test]
fn write_xobject_mediabox_1() {
    run_write_test(
        "write_xobject_mediabox_1",
        "pdfs/page_media_box_bottom_left.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_mediabox_2() {
    run_write_test(
        "write_xobject_mediabox_2",
        "pdfs/page_media_box_top_left.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_mediabox_3() {
    run_write_test(
        "write_xobject_mediabox_3",
        "pdfs/page_media_box_zoomed_out.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_none() {
    run_write_test(
        "write_xobject_rotation_none",
        "pdfs/page_rotation_none.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_90() {
    run_write_test(
        "write_xobject_rotation_90",
        "pdfs/page_rotation_90.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_180() {
    run_write_test(
        "write_xobject_rotation_180",
        "pdfs/page_rotation_180.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_270() {
    run_write_test(
        "write_xobject_rotation_270",
        "pdfs/page_rotation_270.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}
