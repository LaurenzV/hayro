//! Regression tests for the cross-render caches: a document rendered
//! repeatedly (a tiled renderer renders each page many times) must decode
//! each image once, and images whose soft mask has different dimensions
//! must not panic the prepared-paint path.

use hayro::hayro_interpret::Cache;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

fn fixture(name: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(format!("tests/fixtures/{name}"))
}

fn non_white(pixmap: &hayro::vello_cpu::Pixmap) -> usize {
    pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .filter(|px| px[..3] != [255, 255, 255])
        .count()
}

#[test]
fn image_pages_are_decoded_once_and_reused() {
    let pdf = Pdf::new(fixture("image.pdf").unwrap()).unwrap();
    let page = pdf.pages().get(0).unwrap();
    let cache = RenderCache::new_shared(Cache::new());
    let interp = InterpreterSettings::default();
    let settings = RenderSettings::default();
    let pixmap_settings = PixmapSettings { x_scale: 2.0, y_scale: 2.0, ..Default::default() };

    let first = render(page, &cache, &interp, &settings, &pixmap_settings);
    let (_, misses) = cache.interpreter_cache().object_cache_handle().budgeted_counters();

    let second = render(page, &cache, &interp, &settings, &pixmap_settings);
    let (hits, misses_after) = cache.interpreter_cache().object_cache_handle().budgeted_counters();

    assert!(misses >= 1, "the image decode must be a miss first");
    assert!(hits >= 1, "the second render must hit the decode cache");
    assert_eq!(misses_after, misses, "no new decode for the same page");

    // Caching memoizes: identical pixels.
    assert_eq!(first.data_as_u8_slice(), second.data_as_u8_slice());
    assert!(non_white(&second) > 500, "the page must actually have image content");
}

#[test]
fn mismatched_alpha_dimensions_render_without_panicking() {
    let pdf = Pdf::new(fixture("mismatched_alpha.pdf").unwrap()).unwrap();
    let page = pdf.pages().get(0).unwrap();
    let cache = RenderCache::new_shared(Cache::new());
    let interp = InterpreterSettings::default();
    let settings = RenderSettings::default();
    let pixmap_settings = PixmapSettings { x_scale: 2.0, y_scale: 2.0, ..Default::default() };

    let first = render(page, &cache, &interp, &settings, &pixmap_settings);
    let second = render(page, &cache, &interp, &settings, &pixmap_settings);

    assert!(non_white(&second) > 500, "the composited image must be visible");
    assert_eq!(first.data_as_u8_slice(), second.data_as_u8_slice());
}
