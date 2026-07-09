use crate::{get_diff, interpreter_settings, load_pdf, run_write_test};
use hayro_syntax::Pdf;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::GROUP;
use hayro_write::ExtractionQuery;
use image::load_from_memory;
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

// `/OC` entries must survive extraction. Optional content that is hidden in the
// original document (a common example are scanned documents that gate a fallback
// image behind an OCMD with an `AllOff` visibility policy) would otherwise
// become visible in the extracted page.
#[test]
fn write_page_preserves_optional_content() {
    use pdf_writer::{Content, Finish, Name, Rect, Str};

    let original = {
        let mut pdf = pdf_writer::Pdf::new();

        let catalog_id = Ref::new(1);
        let page_tree_id = Ref::new(2);
        let page_id = Ref::new(3);
        let content_id = Ref::new(4);
        let visible_image_id = Ref::new(5);
        let hidden_image_id = Ref::new(6);
        let ocg_id = Ref::new(7);
        let ocmd_id = Ref::new(8);

        let mut catalog = pdf.catalog(catalog_id);
        catalog.pages(page_tree_id);
        let mut oc_properties = catalog.insert(Name(b"OCProperties")).dict();
        oc_properties.insert(Name(b"OCGs")).array().item(ocg_id);
        oc_properties.insert(Name(b"D")).dict();
        oc_properties.finish();
        catalog.finish();

        pdf.pages(page_tree_id).kids([page_id]).count(1);

        let mut page = pdf.page(page_id);
        page.parent(page_tree_id)
            .media_box(Rect::new(0.0, 0.0, 100.0, 100.0))
            .contents(content_id);
        let mut resources = page.resources();
        let mut x_objects = resources.x_objects();
        x_objects.pair(Name(b"Im1"), visible_image_id);
        x_objects.pair(Name(b"Im2"), hidden_image_id);
        x_objects.finish();
        resources.finish();
        page.finish();

        let mut content = Content::new();
        // Draw the visible image over the whole page, and the hidden one on top of it.
        for name in [b"Im1", b"Im2"] {
            content.save_state();
            content.transform([100.0, 0.0, 0.0, 100.0, 0.0, 0.0]);
            content.x_object(Name(name));
            content.restore_state();
        }
        pdf.stream(content_id, &content.finish());

        // A 1x1 white RGB image.
        let mut visible_image = pdf.image_xobject(visible_image_id, &[255, 255, 255]);
        visible_image.width(1).height(1).bits_per_component(8);
        visible_image.color_space().device_rgb();
        visible_image.finish();

        // A 1x1 black RGB image that is only visible if the OCG is disabled,
        // i.e. it is hidden in the default configuration.
        let mut hidden_image = pdf.image_xobject(hidden_image_id, &[0, 0, 0]);
        hidden_image.width(1).height(1).bits_per_component(8);
        hidden_image.color_space().device_rgb();
        hidden_image.pair(Name(b"OC"), ocmd_id);
        hidden_image.finish();

        let mut ocmd = pdf.indirect(ocmd_id).dict();
        ocmd.pair(Name(b"Type"), Name(b"OCMD"));
        ocmd.insert(Name(b"OCGs")).array().item(ocg_id);
        ocmd.pair(Name(b"P"), Name(b"AllOff"));
        ocmd.finish();

        let mut ocg = pdf.indirect(ocg_id).dict();
        ocg.pair(Name(b"Type"), Name(b"OCG"));
        ocg.pair(Name(b"Name"), Str(b"fallback"));
        ocg.finish();

        pdf.finish()
    };

    let hayro_pdf = Pdf::new(original).unwrap();
    let extracted = hayro_write::extract_pages_to_pdf(&hayro_pdf, &[0]);
    let reread = Pdf::new(extracted).unwrap();

    // The `/OC` entry of the image must survive the extraction.
    let image = reread.pages()[0]
        .resources()
        .x_objects
        .get::<Stream<'_>>("Im2")
        .unwrap();
    let ocmd = image
        .dict()
        .get::<hayro_syntax::object::Dict<'_>>("OC")
        .expect("`/OC` entry was dropped during extraction");
    assert_eq!(
        ocmd.get::<hayro_syntax::object::Name<'_>>("P")
            .unwrap()
            .as_ref(),
        b"AllOff"
    );

    let render = |pdf: &Pdf| {
        let mut pages = hayro::render_pdf(pdf, 1.0, interpreter_settings(), None).unwrap();
        load_from_memory(&pages.remove(0).into_png().unwrap())
            .unwrap()
            .into_rgba8()
    };

    let original_render = render(&hayro_pdf);
    let extracted_render = render(&reread);

    // Sanity check: the hidden image must not show up in the original render.
    assert_eq!(
        original_render.get_pixel(50, 50),
        &image::Rgba([255, 255, 255, 255])
    );

    // The extracted page must render identically to the original one. Without
    // the `/OC` entry, the hidden image would become visible.
    let (_, pixel_diff) = get_diff(&original_render, &extracted_render);
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
