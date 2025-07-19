//! This example shows how you can iterate over the content stream of all pages in the PDF.

use hayro_syntax::Pdf;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    eprintln!(
        "{:?}",
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-render/pdfs/text_with_rise.pdf")
    );
    let data = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-render/pdfs/text_with_rise.pdf"),
    )
    .unwrap();
    let pdf = Pdf::new(Arc::new(data)).unwrap();
    let pages = pdf.pages();

    for page in pages.iter() {
        for op in page.typed_operations() {
            println!("{op:?}");
        }
    }
}
