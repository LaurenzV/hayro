//! This example shows how you can iterate over the content stream of all pages in a PDF.

use hayro_syntax::Pdf;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    // First load the data that constitutes the PDF file.
    let data = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro/pdfs/text_with_rise.pdf"),
    )
    .unwrap();

    // Then create a new PDF file from it.
    //
    // Here we are just unwrapping in case reading the file failed, but you
    // might instead want to apply proper error handling.
    let pdf = Pdf::new(Arc::new(data)).unwrap();

    // First access all pages, and then iterate over the operators of each page's
    // content stream and print them.
    let pages = pdf.pages();
    for page in pages.iter() {
        for op in page.typed_operations() {
            println!("{op:?}");
        }
    }
}
