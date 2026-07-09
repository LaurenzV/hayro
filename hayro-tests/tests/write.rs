use crate::{load_pdf, run_write_test};
use hayro_syntax::Pdf;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{ANNOTS, AP, GROUP, N, P, RECT, SUBTYPE};
use hayro_write::ExtractionQuery;
use pdf_writer::Ref;
use sitro::Renderer;

#[test]
fn write_page_basic_1() {
    run_write_test(
        "write_page_basic_1",
        "pdfs/custom/clip_path_evenodd.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn dont_cache_page_references() {
    let hayro_pdf = load_pdf("pdfs/custom/clip_path_evenodd.pdf");
    let mut next_ref = Ref::new(1);
    let extracted = hayro_write::extract(
        &hayro_pdf,
        Box::new(|| next_ref.bump()),
        hayro_write::ChunkSettings::default(),
        |_| {},
        &[ExtractionQuery::new_page(0), ExtractionQuery::new_page(0)],
    )
    .unwrap();

    // Adobe Acrobat does not seem to like reusing the same page reference, so we must always
    // create a new one and not cache them.
    assert_ne!(
        extracted.root_refs[0].unwrap(),
        extracted.root_refs[1].unwrap()
    );
}

#[test]
fn write_page_basic_2() {
    run_write_test(
        "write_page_basic_2",
        "pdfs/custom/integration_coat_of_arms.pdf",
        &[0],
        Renderer::Mupdf,
        true,
    );
}

#[test]
fn write_page_basic_with_xobject() {
    run_write_test(
        "write_page_basic_with_xobject",
        "pdfs/custom/xobject_1.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_basic_with_text() {
    run_write_test(
        "write_page_basic_with_text",
        "pdfs/custom/pdftc_900k_0156_page_2.pdf",
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
        "pdfs/custom/integration_diagram.pdf",
        &[0, 0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_mediabox_1() {
    run_write_test(
        "write_page_mediabox_1",
        "pdfs/custom/page_media_box_bottom_left.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_rotation() {
    run_write_test(
        "write_page_rotation",
        "pdfs/custom/page_rotation_270.pdf",
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
        "downloads/pdfjs/issue17065.pdf",
        &[0],
        Renderer::Pdfium,
        true,
    );
}

#[test]
fn write_page_with_encryption_1() {
    run_write_test(
        "write_page_with_encryption_1",
        "downloads/custom/issue10_1.pdf",
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
        "pdfs/custom/clip_path_evenodd.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_uses_isolated_transparency_group() {
    let hayro_pdf = load_pdf("pdfs/custom/clip_path_evenodd.pdf");
    let extracted = hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, &[0]);
    let rewritten = Pdf::new(extracted).unwrap();
    let page = &rewritten.pages()[0];
    let x_object = page.resources().x_objects.get::<Stream<'_>>("O1").unwrap();
    let group = x_object
        .dict()
        .get::<hayro_syntax::object::Dict<'_>>(GROUP)
        .unwrap();

    assert_eq!(
        group
            .get::<hayro_syntax::object::Name<'_>>("Type")
            .unwrap()
            .as_ref(),
        b"Group"
    );
    assert_eq!(
        group
            .get::<hayro_syntax::object::Name<'_>>("S")
            .unwrap()
            .as_ref(),
        b"Transparency"
    );
    assert_eq!(group.get::<bool>(b"I"), Some(true));
    assert_eq!(
        group
            .get::<hayro_syntax::object::Name<'_>>("CS")
            .unwrap()
            .as_ref(),
        b"DeviceRGB"
    );
}

#[test]
fn write_xobject_basic_2() {
    run_write_test(
        "write_xobject_basic_2",
        "pdfs/custom/integration_coat_of_arms.pdf",
        &[0],
        Renderer::Mupdf,
        false,
    );
}

#[test]
fn write_xobject_mediabox_1() {
    run_write_test(
        "write_xobject_mediabox_1",
        "pdfs/custom/page_media_box_bottom_left.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_mediabox_2() {
    run_write_test(
        "write_xobject_mediabox_2",
        "pdfs/custom/page_media_box_top_left.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_mediabox_3() {
    run_write_test(
        "write_xobject_mediabox_3",
        "pdfs/custom/page_media_box_zoomed_out.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_none() {
    run_write_test(
        "write_xobject_rotation_none",
        "pdfs/custom/page_rotation_none.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_90() {
    run_write_test(
        "write_xobject_rotation_90",
        "pdfs/custom/page_rotation_90.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_180() {
    run_write_test(
        "write_xobject_rotation_180",
        "pdfs/custom/page_rotation_180.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_270() {
    run_write_test(
        "write_xobject_rotation_270",
        "pdfs/custom/page_rotation_270.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_rotation_and_cropbox() {
    run_write_test(
        "write_xobject_rotation_and_cropbox",
        "downloads/pdfbox/1697.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

#[test]
fn write_xobject_contents_array() {
    run_write_test(
        "write_xobject_contents_array",
        "downloads/pdfbox/1084.pdf",
        &[0],
        Renderer::Pdfium,
        false,
    );
}

/// Build a minimal single-page PDF containing a `Square` annotation whose appearance
/// stream draws a black square, and whose `/P` entry points back to the page.
fn pdf_with_annotation() -> Vec<u8> {
    use pdf_writer::types::AnnotationType;
    use pdf_writer::{Content, Finish, Rect};

    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);
    let annot_id = Ref::new(5);
    let appearance_id = Ref::new(6);

    let mut pdf = pdf_writer::Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id).kids([page_id]).count(1);

    let mut page = pdf.page(page_id);
    page.media_box(Rect::new(0.0, 0.0, 200.0, 200.0));
    page.parent(page_tree_id);
    page.contents(content_id);
    page.annotations([annot_id]);
    page.finish();

    pdf.stream(content_id, &Content::new().finish());

    let mut annot = pdf.annotation(annot_id);
    annot.subtype(AnnotationType::Square);
    annot.rect(Rect::new(50.0, 50.0, 150.0, 150.0));
    annot.page(page_id);
    annot.appearance().normal().stream(appearance_id);
    annot.finish();

    let mut appearance = Content::new();
    appearance.rect(0.0, 0.0, 100.0, 100.0);
    appearance.fill_nonzero();
    let appearance_content = appearance.finish();
    pdf.form_xobject(appearance_id, &appearance_content)
        .bbox(Rect::new(0.0, 0.0, 100.0, 100.0));

    pdf.finish()
}

#[test]
fn write_page_with_annotation() {
    let original = Pdf::new(pdf_with_annotation()).unwrap();
    let extracted = hayro_write::extract_pages_to_pdf(&original, &[0]);
    let extracted = Pdf::new(extracted).unwrap();

    // The annotation and its appearance stream must have been copied over.
    let annot = extracted.pages()[0]
        .raw()
        .get::<hayro_syntax::object::Array>(ANNOTS)
        .expect("extracted page should have an `Annots` entry")
        .iter::<hayro_syntax::object::Dict>()
        .next()
        .expect("`Annots` should contain the annotation");
    assert_eq!(
        annot
            .get::<hayro_syntax::object::Name>(SUBTYPE)
            .unwrap()
            .as_ref(),
        b"Square"
    );
    assert!(annot.get::<hayro_syntax::object::Rect>(RECT).is_some());
    assert!(
        annot
            .get::<hayro_syntax::object::Dict>(AP)
            .unwrap()
            .get::<Stream>(N)
            .is_some()
    );

    // The `/P` entry must not have pulled the original page (tree) into the
    // extracted document.
    assert!(annot.get::<hayro_syntax::object::Dict>(P).is_none());

    // The annotation must still render in the extracted document.
    let original_rendered = crate::render_pdf(&original, "", crate::interpreter_settings(), None);
    let extracted_rendered = crate::render_pdf(&extracted, "", crate::interpreter_settings(), None);

    let original_image = image::load_from_memory(&original_rendered[0])
        .unwrap()
        .into_rgba8();
    let extracted_image = image::load_from_memory(&extracted_rendered[0])
        .unwrap()
        .into_rgba8();

    // Make sure we don't just compare two blank pages.
    assert!(
        original_image
            .pixels()
            .any(|pixel| pixel.0 == [0, 0, 0, 255])
    );

    let (_, pixel_diff) = crate::get_diff(&original_image, &extracted_image);
    assert_eq!(pixel_diff, 0);
}

#[test]
fn write_null_objects() {
    let hayro_pdf = load_pdf("pdfs/other/issue188.pdf");
    // Ghostscript still complains that this is an invalid PDF since a dictionary is expected
    // for fonts, but not sure if there is anything better we can do in the first place if the
    // object doesn't exist at all / is invalid.
    let extracted = hayro_write::extract_pages_to_pdf(&hayro_pdf, &[0]);

    let reread = Pdf::new(extracted).unwrap();
    let dict = reread.pages()[0]
        .raw()
        .get::<hayro_syntax::object::Dict>("Resources")
        .unwrap()
        .get::<hayro_syntax::object::Dict>("Font")
        .unwrap();
    let data = dict.data();

    assert_eq!(data, b"<<\n      /F1 5 0 R\n      /F2 null\n    >>");
}
