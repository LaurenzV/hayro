//! Stop latency of `render_with_stop` on real PDFs, at deadlines that fall at
//! various points within the render.
//!
//! ```text
//! cargo run -p hayro --release --example cancel_latency -- <pdf>...
//! ```
//!
//! For each PDF it picks the slowest of the first few pages, measures the full
//! (warm) render time, then fires a deadline stop at 10/25/50/75/90% of that
//! and reports the overshoot (time from the deadline to `render_with_stop`
//! actually returning).

use std::sync::Arc;
use std::time::{Duration, Instant};

use almost_enough::FnStop;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::{RenderCache, RenderSettings, render, render_with_stop};

fn deadline_stop(d: Duration) -> Arc<dyn enough::Stop> {
    let start = Instant::now();
    Arc::new(FnStop::new(move || start.elapsed() >= d))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let scale: f32 = std::env::var("HAYRO_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let rs = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        ..RenderSettings::default()
    };
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let pdf = match Pdf::new(data) {
            Ok(p) => p,
            _ => {
                eprintln!("parse fail: {path}");
                continue;
            }
        };
        let pages = pdf.pages();
        if pages.is_empty() {
            continue;
        }
        let name: String = path.rsplit('/').next().unwrap().chars().take(32).collect();
        let cache = RenderCache::new();
        // Slowest of the first few pages (cold renders also warm the cache).
        let n = pages.len().min(4);
        let (mut pi, mut slow) = (0_usize, Duration::ZERO);
        for i in 0..n {
            let t = Instant::now();
            let _ = render(&pages[i], &cache, &InterpreterSettings::default(), &rs);
            let dt = t.elapsed();
            if dt > slow {
                slow = dt;
                pi = i;
            }
        }
        let page = &pages[pi];
        // Warm full render time (cache already warm from the find pass).
        let t = Instant::now();
        let _ = render(page, &cache, &InterpreterSettings::default(), &rs);
        let full = t.elapsed();
        println!(
            "\n{name}  page {pi}/{}  full {:.1} ms @{scale}x",
            pages.len(),
            ms(full)
        );
        println!(
            "  {:>6} {:>10} {:>11} {:>6}",
            "frac", "deadline", "overshoot", "stop?"
        );
        for frac in [0.10_f64, 0.25, 0.50, 0.75, 0.90] {
            let deadline = full.mul_f64(frac);
            let s = InterpreterSettings {
                stop: deadline_stop(deadline),
                ..InterpreterSettings::default()
            };
            let t = Instant::now();
            let r = render_with_stop(page, &cache, &s, &rs);
            let elapsed = t.elapsed();
            let overshoot = elapsed.saturating_sub(deadline);
            println!(
                "  {:>5.0}% {:>8.1}ms {:>9.1}ms {:>6}",
                frac * 100.0,
                ms(deadline),
                ms(overshoot),
                if r.is_err() { "yes" } else { "no" }
            );
        }
    }
}
