use hayro::InterpreterSettings;
use hayro::Pdf;
use hayro::render_pdf;
use std::sync::Arc;

fn load(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    if let Ok(pdf) = pdf {
        let _pixmaps = render_pdf(&pdf, 1.0, InterpreterSettings::default(), None);
    }
}

#[test]
fn issue50() {
    let file = include_bytes!("../pdfs/load/issue50.pdf");
    load(file);
}

#[test]
fn issue54() {
    let file = include_bytes!("../pdfs/load/issue54.pdf");
    load(file);
}

#[test]
fn issue55() {
    let file = include_bytes!("../pdfs/load/issue55.pdf");
    load(file);
}

#[test]
fn issue56() {
    let file = include_bytes!("../pdfs/load/issue56.pdf");
    load(file);
}

#[test]
fn issue61() {
    let file = include_bytes!("../pdfs/load/issue61.pdf");
    load(file);
}

#[test]
fn issue62() {
    let file = include_bytes!("../pdfs/load/issue62.pdf");
    load(file);
}

#[test]
fn issue63() {
    let file = include_bytes!("../pdfs/load/issue63.pdf");
    load(file);
}

#[test]
fn issue67() {
    let file = include_bytes!("../pdfs/load/issue67.pdf");
    load(file);
}

#[test]
fn issue68() {
    let file = include_bytes!("../pdfs/load/issue68.pdf");
    load(file);
}

#[test]
fn issue83() {
    let file = include_bytes!("../pdfs/load/issue83.pdf");
    load(file);
}

#[test]
fn issue152() {
    let file = include_bytes!("../pdfs/load/issue152.pdf");
    load(file);
}

#[test]
fn issue153() {
    let file = include_bytes!("../pdfs/load/issue153.pdf");
    load(file);
}

#[test]
fn issue154() {
    let file = include_bytes!("../pdfs/load/issue154.pdf");
    load(file);
}

#[test]
fn issue157() {
    let file = include_bytes!("../pdfs/load/issue157.pdf");
    load(file);
}

#[test]
fn issue178() {
    let file = include_bytes!("../pdfs/load/issue178.pdf");
    load(file);
}

#[test]
fn issue180() {
    let file = include_bytes!("../pdfs/load/issue180.pdf");
    load(file);
}

#[test]
fn issue182() {
    let file = include_bytes!("../pdfs/load/issue182.pdf");
    load(file);
}

#[test]
fn issue203() {
    let file = include_bytes!("../pdfs/load/issue203.pdf");
    load(file);
}

#[test]
fn issue204() {
    let file = include_bytes!("../pdfs/load/issue204.pdf");
    load(file);
}

#[test]
fn issue205() {
    let file = include_bytes!("../pdfs/load/issue205.pdf");
    load(file);
}

#[test]
fn issue206() {
    let file = include_bytes!("../pdfs/load/issue206.pdf");
    load(file);
}

#[test]
fn issue207() {
    let file = include_bytes!("../pdfs/load/issue207.pdf");
    load(file);
}

#[test]
fn issue208() {
    let file = include_bytes!("../pdfs/load/issue208.pdf");
    load(file);
}

#[test]
fn issue222() {
    let file = include_bytes!("../pdfs/load/issue222.pdf");
    load(file);
}

#[test]
fn issue223() {
    let file = include_bytes!("../pdfs/load/issue223.pdf");
    load(file);
}

#[test]
fn issue224() {
    let file = include_bytes!("../pdfs/load/issue224.pdf");
    load(file);
}

#[test]
fn issue234() {
    let file = include_bytes!("../pdfs/load/issue234.pdf");
    load(file);
}

#[test]
fn issue235() {
    let file = include_bytes!("../pdfs/load/issue235.pdf");
    load(file);
}

#[test]
fn issue236() {
    let file = include_bytes!("../pdfs/load/issue236.pdf");
    load(file);
}

#[test]
fn issue273_180b() {
    let file = include_bytes!("../pdfs/load/issue273_180b.pdf");
    load(file);
}

#[test]
fn issue323() {
    let file = include_bytes!("../pdfs/load/issue323.pdf");
    load(file);
}

#[test]
fn issue324() {
    let file = include_bytes!("../pdfs/load/issue324.pdf");
    load(file);
}

#[test]
fn issue325() {
    let file = include_bytes!("../pdfs/load/issue325.pdf");
    load(file);
}

#[test]
fn issue351() {
    let file = include_bytes!("../pdfs/load/issue351.pdf");
    load(file);
}

#[test]
fn issue352() {
    let file = include_bytes!("../pdfs/load/issue352.pdf");
    load(file);
}

#[test]
fn issue355() {
    let file = include_bytes!("../pdfs/load/issue355.pdf");
    load(file);
}

#[test]
fn issue356() {
    let file = include_bytes!("../pdfs/load/issue356.pdf");
    load(file);
}

#[test]
fn issue357() {
    let file = include_bytes!("../pdfs/load/issue357.pdf");
    load(file);
}

#[test]
fn issue372() {
    let file = include_bytes!("../pdfs/load/issue372.pdf");
    load(file);
}

#[test]
fn issue390() {
    let file = include_bytes!("../pdfs/load/issue390.pdf");
    load(file);
}

#[test]
fn issue391() {
    let file = include_bytes!("../pdfs/load/issue391.pdf");
    load(file);
}

#[test]
fn issue409() {
    let file = include_bytes!("../pdfs/load/issue409.pdf");
    load(file);
}

#[test]
fn page_tree_cycle() {
    let file = include_bytes!("../pdfs/load/page_tree_cycle.pdf");
    load(file);
}
