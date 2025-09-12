use hayro::InterpreterSettings;
use hayro::Pdf;
use hayro::render_pdf;
use std::sync::Arc;

fn render(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    // Just make sure we don't crash.
    if let Ok(pdf) = pdf {
        let _pixmaps = render_pdf(&pdf, 1.0, InterpreterSettings::default(), None);
    }
}

#[test]
fn issue54() {
    let file = include_bytes!("../pdfs/crash/issue54.pdf");
    render(file);
}

#[test]
fn issue55() {
    let file = include_bytes!("../pdfs/crash/issue55.pdf");
    render(file);
}

#[test]
fn issue56() {
    let file = include_bytes!("../pdfs/crash/issue56.pdf");
    render(file);
}

#[test]
fn issue61() {
    let file = include_bytes!("../pdfs/crash/issue61.pdf");
    render(file);
}

#[test]
fn issue62() {
    let file = include_bytes!("../pdfs/crash/issue62.pdf");
    render(file);
}

#[test]
fn issue67() {
    let file = include_bytes!("../pdfs/crash/issue67.pdf");
    render(file);
}

#[test]
fn issue68() {
    let file = include_bytes!("../pdfs/crash/issue68.pdf");
    render(file);
}

#[test]
fn issue152() {
    let file = include_bytes!("../pdfs/crash/issue152.pdf");
    render(file);
}

#[test]
fn issue153() {
    let file = include_bytes!("../pdfs/crash/issue153.pdf");
    render(file);
}

#[test]
fn issue157() {
    let file = include_bytes!("../pdfs/crash/issue157.pdf");
    render(file);
}

#[test]
fn issue180() {
    let file = include_bytes!("../pdfs/crash/issue180.pdf");
    render(file);
}

#[test]
fn issue182() {
    let file = include_bytes!("../pdfs/crash/issue182.pdf");
    render(file);
}

#[test]
fn issue203() {
    let file = include_bytes!("../pdfs/crash/issue203.pdf");
    render(file);
}

#[test]
fn issue204() {
    let file = include_bytes!("../pdfs/crash/issue204.pdf");
    render(file);
}

#[test]
fn issue205() {
    let file = include_bytes!("../pdfs/crash/issue205.pdf");
    render(file);
}

#[test]
fn issue206() {
    let file = include_bytes!("../pdfs/crash/issue206.pdf");
    render(file);
}

#[test]
fn issue207() {
    let file = include_bytes!("../pdfs/crash/issue207.pdf");
    render(file);
}

#[test]
fn issue208() {
    let file = include_bytes!("../pdfs/crash/issue208.pdf");
    render(file);
}

#[test]
fn issue222() {
    let file = include_bytes!("../pdfs/crash/issue222.pdf");
    render(file);
}

#[test]
fn issue223() {
    let file = include_bytes!("../pdfs/crash/issue223.pdf");
    render(file);
}

#[test]
fn issue235() {
    let file = include_bytes!("../pdfs/crash/issue235.pdf");
    render(file);
}

#[test]
fn issue236() {
    let file = include_bytes!("../pdfs/crash/issue236.pdf");
    render(file);
}
