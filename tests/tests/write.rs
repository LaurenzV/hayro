use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{interpreter_settings, load_pdf, render_pdf, try_load_pdf};
use hayro_syntax::Pdf;
use hayro_syntax::object::dict::keys::GROUP;
use hayro_syntax::object::{Dict, Stream};
use hayro_write::ExtractionQuery;
use pdf_writer::Ref;

#[derive(Clone, Copy)]
enum Kind {
    Page,
    XObject,
}

type WriteTest = (
    &'static str,
    &'static str,
    &'static str,
    &'static [usize],
    Kind,
);

const TESTS: &[WriteTest] = &[
    (
        "write_page_basic_1",
        "write_page_basic_1",
        "pdfs/custom/clip_path_evenodd.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_basic_2",
        "write_page_basic_2",
        "pdfs/custom/integration_coat_of_arms.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_basic_with_xobject",
        "write_page_basic_with_xobject",
        "pdfs/custom/xobject_1.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_basic_with_text",
        "write_page_basic_with_text",
        "pdfs/custom/pdftc_900k_0156_page_2.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_with_shading",
        "write_page_shading",
        "downloads/pdfbox/1915_17.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_duplicated_page",
        "write_page_duplicated_page",
        "pdfs/custom/integration_diagram.pdf",
        &[0, 0],
        Kind::Page,
    ),
    (
        "write_page_mediabox_1",
        "write_page_mediabox_1",
        "pdfs/custom/page_media_box_bottom_left.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_rotation",
        "write_page_rotation",
        "pdfs/custom/page_rotation_270.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_multiple_pages_1",
        "write_page_multiple_pages_1",
        "downloads/pdfbox/1772.pdf",
        &[0, 2, 1, 6, 8, 0],
        Kind::Page,
    ),
    (
        "write_page_multiple_pages_2",
        "write_page_multiple_pages_2",
        "downloads/pdfbox/2191.pdf",
        &[0, 1, 7],
        Kind::Page,
    ),
    // Original PDF contains reference for `ToUnicode`, but doesn't actually have it in the PDF.
    (
        "write_page_missing_ref",
        "write_page_missing_ref",
        "downloads/pdfbox/5992_1.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_with_inherited_resources_1",
        "write_page_with_inherited_resource",
        "downloads/pdfbox/5910.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_with_inherited_resources_2",
        "write_page_with_inherited_resources_2",
        "downloads/pdfjs/issue17065.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_page_with_encryption_1",
        "write_page_with_encryption_1",
        "downloads/custom/issue10_1.pdf",
        &[0],
        Kind::Page,
    ),
    // This PDF uses named property lists in `BDC` operators, so its `Properties` resources must be
    // preserved. Omitting them causes rendering issues in Quartz and a warning in Ghostscript.
    (
        "write_page_with_properties",
        "write_page_with_properties",
        "downloads/pdfbox/3754.pdf",
        &[0],
        Kind::Page,
    ),
    (
        "write_xobject_basic_1",
        "write_xobject_basic_1",
        "pdfs/custom/clip_path_evenodd.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_basic_2",
        "write_xobject_basic_2",
        "pdfs/custom/integration_coat_of_arms.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_mediabox_1",
        "write_xobject_mediabox_1",
        "pdfs/custom/page_media_box_bottom_left.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_mediabox_2",
        "write_xobject_mediabox_2",
        "pdfs/custom/page_media_box_top_left.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_mediabox_3",
        "write_xobject_mediabox_3",
        "pdfs/custom/page_media_box_zoomed_out.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_rotation_none",
        "write_xobject_rotation_none",
        "pdfs/custom/page_rotation_none.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_rotation_90",
        "write_xobject_rotation_90",
        "pdfs/custom/page_rotation_90.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_rotation_180",
        "write_xobject_rotation_180",
        "pdfs/custom/page_rotation_180.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_rotation_270",
        "write_xobject_rotation_270",
        "pdfs/custom/page_rotation_270.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_rotation_and_cropbox",
        "write_xobject_rotation_and_cropbox",
        "downloads/pdfbox/1697.pdf",
        &[0],
        Kind::XObject,
    ),
    (
        "write_xobject_contents_array",
        "write_xobject_contents_array",
        "downloads/pdfbox/1084.pdf",
        &[0],
        Kind::XObject,
    ),
];

pub(crate) fn tests() -> Vec<Test> {
    let snapshots = TESTS.iter().map(|&(name, snapshot, file, pages, kind)| {
        let mut files = vec![file.to_string()];
        files.extend(FALLBACK_FONTS.map(String::from));

        Test::new(format!("write::{name}"), files, move || {
            let hayro_pdf = match try_load_pdf(file, "") {
                Ok(pdf) => pdf,
                Err(err) => return Outcome::Failed(err),
            };
            let buf = match kind {
                Kind::Page => hayro_write::extract_pages_to_pdf(&hayro_pdf, pages),
                Kind::XObject => hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, pages),
            };

            if crate::harness::args::args().store {
                let dir = crate::WORKSPACE_PATH.join("store");
                let _ = std::fs::create_dir_all(&dir);
                std::fs::write(dir.join(format!("{snapshot}.pdf")), &buf).unwrap();
            }

            let pdf = Pdf::new(buf).unwrap();
            let rendered = render_pdf(&pdf, snapshot, interpreter_settings(), None);
            check(Suite::Write, snapshot, rendered)
        })
    });

    let units: [(&str, fn()); 4] = [
        (
            "write::dont_cache_page_references",
            dont_cache_page_references,
        ),
        (
            "write::write_xobject_does_not_synthesize_group",
            write_xobject_does_not_synthesize_group,
        ),
        (
            "write::write_xobject_preserves_page_group",
            write_xobject_preserves_page_group,
        ),
        ("write::write_null_objects", write_null_objects),
    ];

    snapshots
        .chain(units.map(|(name, run)| Test::unit(name, run)))
        .collect()
}

fn dont_cache_page_references() {
    let hayro_pdf = load_pdf("pdfs/custom/clip_path_evenodd.pdf");
    let mut next_ref = Ref::new(1);
    let extracted = hayro_write::extract(
        &hayro_pdf,
        Box::new(|| next_ref.bump()),
        hayro_write::ChunkSettings::default(),
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

// See also https://github.com/LaurenzV/krilla/pull/410 for more info.
fn write_xobject_does_not_synthesize_group() {
    let hayro_pdf = load_pdf("pdfs/custom/clip_path_evenodd.pdf");
    let extracted = hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, &[0]);
    let rewritten = Pdf::new(extracted).unwrap();
    let page = &rewritten.pages()[0];
    let x_object = page.resources().x_objects.get::<Stream<'_>>("O1").unwrap();

    assert!(x_object.dict().get::<Dict<'_>>(GROUP).is_none());
}

// See also https://github.com/LaurenzV/krilla/pull/410 for more info.
fn write_xobject_preserves_page_group() {
    let hayro_pdf = load_pdf("pdfs/custom/shading_operator_2.pdf");
    let extracted = hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, &[0]);
    let rewritten = Pdf::new(extracted).unwrap();
    let page = &rewritten.pages()[0];
    let x_object = page.resources().x_objects.get::<Stream<'_>>("O1").unwrap();
    let group = x_object.dict().get::<Dict<'_>>(GROUP).unwrap();

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
