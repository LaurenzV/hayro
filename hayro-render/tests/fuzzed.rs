use hayro_interpret::InterpreterSettings;
use hayro_render::render_png;
use hayro_syntax::pdf::Pdf;
use std::sync::Arc;

fn render_fuzzed(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    if let Some(pdf) = pdf {
        let _pixmaps = render_png(&pdf, 1.0, InterpreterSettings::default(), None);
    }
}

#[test]
fn issue54() {
    let file = include_bytes!("fuzzed_pdfs/issue54.pdf");
    render_fuzzed(file);
}

#[test]
fn issue55() {
    let file = include_bytes!("fuzzed_pdfs/issue55.pdf");
    render_fuzzed(file);
}

#[test]
fn issue56() {
    let file = include_bytes!("fuzzed_pdfs/issue56.pdf");
    render_fuzzed(file);
}

#[test]
fn issue61() {
    let file = include_bytes!("fuzzed_pdfs/issue61.pdf");
    render_fuzzed(file);
}

#[test]
fn issue62() {
    let file = include_bytes!("fuzzed_pdfs/issue62.pdf");
    render_fuzzed(file);
}

#[test]
fn issue67() {
    let file = include_bytes!("fuzzed_pdfs/issue67.pdf");
    render_fuzzed(file);
}

#[test]
fn issue68() {
    let file = include_bytes!("fuzzed_pdfs/issue68.pdf");
    render_fuzzed(file);
}
