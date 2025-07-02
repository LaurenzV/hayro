use hayro_render::render_png;
use hayro_syntax::pdf::Pdf;
use std::sync::Arc;

fn render_fuzzed(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    if let Some(pdf) = pdf {
        let _pixmaps = render_png(&pdf, 1.0, None);
    }
}

#[test]
fn issue_55() {
    let file = include_bytes!("fuzzed_pdfs/issue55.pdf");
    render_fuzzed(file);
}

#[test]
fn issue_56() {
    let file = include_bytes!("fuzzed_pdfs/issue56.pdf");
    render_fuzzed(file);
}

#[test]
fn issue_61() {
    let file = include_bytes!("fuzzed_pdfs/issue61.pdf");
    render_fuzzed(file);
}

#[test]
fn issue_62() {
    let file = include_bytes!("fuzzed_pdfs/issue62.pdf");
    render_fuzzed(file);
}
