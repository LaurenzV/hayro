//! Cooperative-cancellation latency tests for `render_with_stop`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use almost_enough::FnStop;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::{RenderCache, RenderSettings, render, render_with_stop};

/// Build a minimal valid PDF whose single page contains `n` fill operations,
/// so interpretation time dominates rendering and is proportional to `n`.
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

fn deadline_stop(deadline: Duration) -> Arc<dyn enough::Stop> {
    let start = Instant::now();
    Arc::new(FnStop::new(move || start.elapsed() >= deadline))
}

#[test]
fn stop_bounds_render_latency() {
    // Enough operators that a full render takes hundreds of ms, dominated by
    // interpretation (one fill op per rectangle), at scale 1 so pixmap setup
    // stays cheap.
    let pdf = Pdf::new(synthetic_pdf(400_000)).unwrap();
    let page = &pdf.pages()[0];
    let cache = RenderCache::new();
    let render_settings = RenderSettings::default();

    let settings = InterpreterSettings::default();
    let start = Instant::now();
    let _ = render(page, &cache, &settings, &render_settings);
    let full = start.elapsed();
    assert!(
        full >= Duration::from_millis(200),
        "synthetic render too fast to test cancellation ({full:?})"
    );

    let deadline = Duration::from_millis(50);
    let stop_settings = InterpreterSettings {
        stop: deadline_stop(deadline),
        ..InterpreterSettings::default()
    };

    let start = Instant::now();
    let result = render_with_stop(page, &cache, &stop_settings, &render_settings);
    let elapsed = start.elapsed();

    assert!(result.is_err(), "stop did not fire (took {elapsed:?})");
    let overshoot = elapsed.saturating_sub(deadline);
    println!("full render {full:?}; cancelled after {elapsed:?} (overshoot {overshoot:?})");
    // Target: <= 10ms between the deadline passing and render returning.
    // Asserted with some headroom for noisy CI machines.
    assert!(
        overshoot <= Duration::from_millis(25),
        "cancellation overshoot too large: {overshoot:?}"
    );
}

#[test]
fn unstoppable_stop_renders_fine() {
    let pdf = Pdf::new(synthetic_pdf(500)).unwrap();
    let page = &pdf.pages()[0];
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let render_settings = RenderSettings::default();

    let pixmap = render_with_stop(page, &cache, &settings, &render_settings).unwrap();
    let plain = render(page, &cache, &settings, &render_settings);
    assert_eq!(pixmap.width(), plain.width());
    assert_eq!(pixmap.height(), plain.height());
}
