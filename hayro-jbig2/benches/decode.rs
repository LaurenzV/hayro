#![allow(missing_docs)]

use std::fs;

use criterion::{Criterion, criterion_group, criterion_main};
use hayro_jbig2::{DecoderContext, Image};

struct NullDecoder;

impl hayro_jbig2::Decoder for NullDecoder {
    fn push_pixel(&mut self, _black: bool) {}
    fn push_pixel_chunk(&mut self, _black: bool, _chunk_count: u32) {}
    fn next_line(&mut self) {}
}

/// Files marked as invalid in the manifest.
const IGNORED: &[&str] = &["042_13", "042_14"];

fn bench_decode(c: &mut Criterion) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/power_jbig2");

    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("missing test-inputs/power_jbig2 directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            let valid_ext = path
                .extension()
                .is_some_and(|ext| ext == "jb2" || ext == "jbig2");
            let not_ignored = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|name| !IGNORED.contains(&name));
            valid_ext && not_ignored
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let mut group = c.benchmark_group("decode");

    for entry in &entries {
        let path = entry.path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let data = fs::read(&path).unwrap();

        let image = match Image::new(&data) {
            Ok(img) => img,
            Err(_) => continue,
        };

        group.bench_function(&name, |b| {
            let mut ctx = DecoderContext::default();
            b.iter(|| {
                image.decode_with(&mut NullDecoder, &mut ctx).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
