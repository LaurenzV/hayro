//! This example shows how you can iterate over the content stream of all pages in the PDF.

use hayro_syntax::content::TypedIter;
use hayro_syntax::object::number::Number;
use hayro_syntax::pdf::Pdf;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    // eprintln!(
    //     "{:?}",
    //     PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-render/pdfs/text_with_rise.pdf")
    // );
    // let data = std::fs::read(
    //     PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-render/pdfs/text_with_rise.pdf"),
    // )
    // .unwrap();
    // let pdf = Pdf::new(Arc::new(data)).unwrap();
    // let pages = pdf.pages().unwrap();
    //
    // for page in pages.get() {
    //     for op in page.typed_operations() {
    //         println!("{:?}", op);
    //     }
    // }

    use hayro_syntax::content::ops::*;

    let content_stream = b"1 0 0 -1 0 200 cm
0 1.0 0 rg
0 0 m
200 0 l
200 200 l
0 200 l
h
f";

    let mut iter = TypedIter::new(content_stream);
    assert!(matches!(iter.next(), Some(TypedInstruction::Transform(_))));
    assert!(matches!(
        iter.next(),
        Some(TypedInstruction::NonStrokeColorDeviceRgb(_))
    ));
    assert!(matches!(iter.next(), Some(TypedInstruction::MoveTo(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::ClosePath(_))));
    assert!(matches!(
        iter.next(),
        Some(TypedInstruction::FillPathNonZero(_))
    ));
}
