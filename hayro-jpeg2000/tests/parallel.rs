//! Tile parallelism must preserve samples, cancellation, and context reuse.

use crate::*;
use hayro_jpeg2000::{CancelHandle, DecodeError};
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn samples(image: &Image<'_>) -> Result<Vec<Vec<u32>>, DecodeError> {
    let mut ctx = DecoderContext::default();
    Ok(image
        .decode(&mut ctx)?
        .components()
        .iter()
        .map(|component| component.samples().iter().map(|x| x.to_bits()).collect())
        .collect())
}

#[test]
fn bit_exact_across_pools() {
    let pools: Vec<_> = [1, 2, 4]
        .into_iter()
        .map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        })
        .collect();
    for stream in [
        "codestream_tiles_partial",
        "codestream_tiles_offset_many",
        "codestream_subsampling_all_components",
        "codestream_subsampling_odd_factors",
        "codestream_tile_cod_fewer_levels",
        "packets_poc_main_header",
        "packets_poc_per_tile",
        "packets_poc_main_header_tile_override",
        "packets_poc_subsampled",
        "packets_poc_offset",
        "packets_ppm_many_tiles",
        "packets_ppt_sop_eph_tiny_tiles",
        "quantization_irreversible_tiles_offset",
        "quantization_irreversible_tiny_tiles",
        "palette_tiled",
        "tile_parts_interleaved",
    ] {
        let data = read_stream(stream);
        for target_resolution in [None, Some((64, 48)), Some((16, 12))] {
            let image = Image::new(
                &data,
                &DecodeSettings {
                    target_resolution,
                    ..Default::default()
                },
            )
            .unwrap();
            let expected = pools[0].install(|| samples(&image));
            if target_resolution.is_none() {
                assert!(expected.is_ok(), "{stream}: {expected:?}");
            }
            for pool in &pools[1..] {
                let actual = pool.install(|| samples(&image));
                assert_eq!(expected, actual, "{stream}, {target_resolution:?}");
            }
        }
    }
}

#[test]
fn nested_parallelism() {
    let data = read_stream("codestream_tiles_partial");
    let image = Image::new(&data, &DecodeSettings::default()).unwrap();
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let expected = serial.install(|| samples(&image));
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    parallel.install(|| {
        (0..8)
            .into_par_iter()
            .for_each(|_| assert_eq!(samples(&image), expected));
    });
}

#[test]
fn cancellation_during_decode_and_context_reuse() {
    let checks = Arc::new(AtomicUsize::new(0));
    let enabled = Arc::new(AtomicBool::new(true));
    let settings = DecodeSettings {
        cancel: Some(CancelHandle::new({
            let checks = checks.clone();
            let enabled = enabled.clone();
            move || enabled.load(Ordering::Relaxed) && checks.fetch_add(1, Ordering::Relaxed) >= 20
        })),
        ..Default::default()
    };
    let data = read_stream("codestream_tiles_offset_many");
    let image = Image::new(&data, &settings).unwrap();
    let mut ctx = DecoderContext::default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        assert!(matches!(
            image.decode(&mut ctx),
            Err(DecodeError::Cancelled)
        ));
        enabled.store(false, Ordering::Relaxed);
        let actual: Vec<_> = image
            .decode(&mut ctx)
            .unwrap()
            .components()
            .iter()
            .map(|c| c.samples().iter().map(|x| x.to_bits()).collect::<Vec<_>>())
            .collect();
        assert_eq!(actual, samples(&image).unwrap());
    });
}

// Run with --release --features threads --test tests benchmark_tiles -- --ignored --nocapture.
#[test]
#[ignore = "set HAYRO_JPEG2000_BENCH_INPUT to a multi-tile JPEG2000 image"]
fn benchmark_tiles() {
    use std::time::Instant;

    let data = std::fs::read(std::env::var("HAYRO_JPEG2000_BENCH_INPUT").unwrap()).unwrap();
    let image = Image::new(&data, &DecodeSettings::default()).unwrap();
    let mut expected = None;
    for threads in [1, 2, 4, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut ctx = DecoderContext::default();
            let mut times = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                let decoded = image.decode(&mut ctx).unwrap();
                times.push(start.elapsed());
                let actual: Vec<Vec<u32>> = decoded
                    .components()
                    .iter()
                    .map(|c| c.samples().iter().map(|x| x.to_bits()).collect())
                    .collect();
                if let Some(expected) = &expected {
                    assert_eq!(&actual, expected);
                } else {
                    expected = Some(actual);
                }
            }
            times.sort();
            println!(
                "{threads} threads: {:?} median ({}x{}, bit-exact)",
                times[1],
                image.width(),
                image.height()
            );
        });
    }
}
