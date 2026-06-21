//! Cooperative-cancellation test for `convert_with_stop`.

use std::sync::Arc;

use hayro_svg::hayro_interpret::InterpreterSettings;
use hayro_svg::hayro_interpret::hayro_syntax::Pdf;
use hayro_svg::{RenderCache, SvgRenderSettings, convert, convert_with_stop};

/// Build a minimal valid PDF whose single page contains `n` fill operations.
fn synthetic_pdf(n: usize) -> Vec<u8> {
    let mut content = String::new();
    for i in 0..n {
        let x = (i % 600) as f32;
        let y = ((i / 600) % 780) as f32;
        content.push_str(&format!("{x} {y} 1.5 1.5 re f\n"));
    }

    let mut pdf = Vec::new();
    let mut offsets = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");

    let obj = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(pdf.len());
        pdf.extend_from_slice(body);
    };

    obj(
        &mut pdf,
        &mut offsets,
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );
    obj(
        &mut pdf,
        &mut offsets,
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    );
    obj(
        &mut pdf,
        &mut offsets,
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
    );
    let stream = format!(
        "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
        content.len(),
        content
    );
    obj(&mut pdf, &mut offsets, stream.as_bytes());

    let xref_pos = pdf.len();
    let mut xref = String::from("xref\n0 5\n0000000000 65535 f \n");
    for off in &offsets {
        xref.push_str(&format!("{off:010} 00000 n \n"));
    }
    pdf.extend_from_slice(xref.as_bytes());
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes(),
    );
    pdf
}

/// A stop that fires immediately, on the very first poll.
struct AlwaysStop;
impl enough::Stop for AlwaysStop {
    fn check(&self) -> Result<(), enough::StopReason> {
        Err(enough::StopReason::Cancelled)
    }
}

#[test]
fn convert_with_stop_aborts() {
    let pdf = Pdf::new(synthetic_pdf(20_000)).unwrap();
    let page = &pdf.pages()[0];
    let cache = RenderCache::new();
    let settings = InterpreterSettings {
        stop: Arc::new(AlwaysStop),
        ..InterpreterSettings::default()
    };

    let result = convert_with_stop(page, &cache, &settings, &SvgRenderSettings::default());
    assert!(
        matches!(result, Err(enough::StopReason::Cancelled)),
        "stop did not propagate to convert_with_stop"
    );
}

#[test]
fn convert_with_stop_unstoppable_matches_convert() {
    let pdf = Pdf::new(synthetic_pdf(200)).unwrap();
    let page = &pdf.pages()[0];
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let render_settings = SvgRenderSettings::default();

    let with_stop = convert_with_stop(page, &cache, &settings, &render_settings).unwrap();
    let plain = convert(page, &cache, &settings, &render_settings);
    assert_eq!(with_stop, plain);
}
