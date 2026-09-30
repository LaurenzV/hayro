//! Extract one page of a PDF.

use hayro_syntax::Pdf;
use hayro_write::{
    ChunkSettings, ExtractionQuery, extract,
    pdf_writer::{self, Ref},
};
use std::path::PathBuf;

fn main() {
    // First load the data that constitutes the PDF file.
    let data = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../in.pdf")).unwrap();
    let page_index = 0;

    // Then create a new PDF file from it.
    //
    // Here we are just unwrapping in case reading the file failed, but you
    // might instead want to apply proper error handling.
    let pdf = Pdf::new(data).unwrap();

    let query = ExtractionQuery::new_page(page_index);

    let mut counter = 0;
    let mut new_ref = || {
        counter += 1;
        Ref::new(counter)
    };
    let extraction_result = extract(
        &pdf,
        Box::new(&mut new_ref),
        ChunkSettings { pretty: false },
        &[query],
    )
    .unwrap();

    let chunk = extraction_result.chunk;
    let page_ref = extraction_result.root_refs[0].unwrap();
    let parent = extraction_result.page_tree_parent_ref;

    let catalog_id = new_ref();
    let mut out_pdf = pdf_writer::Pdf::with_settings(ChunkSettings { pretty: false });
    out_pdf.extend(&chunk);
    out_pdf.catalog(catalog_id).pages(parent);
    out_pdf.pages(parent).kids([page_ref]).count(1);

    std::fs::write(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../out.pdf"),
        out_pdf.finish(),
    )
    .unwrap();
}
