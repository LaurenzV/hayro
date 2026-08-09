//! This example is similar to `iterate`,
//! but it uses a StreamingSource to only read the first page.

use hayro_syntax::{FileSource, Pdf};
use std::path::PathBuf;

fn main() {
    // Open file
    let file = std::fs::File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../in.pdf")).unwrap();
	
	// Create the file-source (implements StreamingSource)
	let file_source = FileSource::new(file, 1024);

    // Then create a new PDF file from it.
    //
    // Here we are just unwrapping in case reading the file failed, but you
    // might instead want to apply proper error handling.
    let pdf = Pdf::new(file_source).unwrap();

    // First access all pages, and then iterate over the operators of each page's
    // content stream and print them.
    let pages = pdf.pages();
	let page = pages.first().unwrap();
	let mut ops = page.typed_operations();

	while let Some(op) = ops.next() {
		println!("{op}");
	}
}
