use hayro_syntax::pdf::Pdf;
use hayro_write::extract_pages;
use pdf_writer::Ref;
use std::env;
use std::sync::Arc;

fn main() {
    let input_path = env::args().nth(1).unwrap();
    let file = std::fs::read(input_path).unwrap();
    let mut hayro_pdf = Pdf::new(Arc::new(file)).unwrap();
    let pages = hayro_pdf.pages().unwrap();
    let page_indices = (0..pages.len()).collect::<Vec<_>>();

    let mut pdf = pdf_writer::Pdf::new();
    let mut next_ref = Ref::new(1);

    let catalog_id = next_ref.bump();
    let page_tree_id = next_ref.bump();
    pdf.catalog(catalog_id).pages(page_tree_id);

    let extracted = extract_pages(&hayro_pdf, next_ref, page_tree_id, &page_indices).unwrap();
    let count = extracted.page_refs.len();
    pdf.pages(page_tree_id)
        .kids(extracted.page_refs)
        .count(count as i32);
    pdf.extend(&extracted.chunk);

    let buf: Vec<u8> = pdf.finish();

    std::fs::write("result.pdf", buf).unwrap();
}
