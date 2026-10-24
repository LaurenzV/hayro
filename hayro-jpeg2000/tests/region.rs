//! Decoding a rectangular region of the image.
//!
//! Instead of comparing against references, every region is compared against
//! the same crop of a full decode, which must be bit-identical.

use crate::*;
use hayro_jpeg2000::{DecodeError, Region, ValidationError};

/// The decoded samples of each component and the image size.
struct Samples {
    width: u32,
    height: u32,
    components: Vec<Vec<f32>>,
    data_u8: Vec<u8>,
}

fn decode_samples(data: &[u8], settings: &DecodeSettings) -> Result<Samples, DecodeError> {
    let image = Image::new(data, settings)?;
    let (width, height) = (image.width(), image.height());
    let mut ctx = DecoderContext::default();
    let decoded = image.decode(&mut ctx)?;

    Ok(Samples {
        width,
        height,
        components: decoded
            .components()
            .iter()
            .map(|c| c.samples().to_vec())
            .collect(),
        data_u8: decoded.data_u8(),
    })
}

/// Tile boundaries in the output coordinate system, read from the `SIZ`
/// marker.
fn tile_boundaries(data: &[u8], width: u32, height: u32) -> (Vec<u32>, Vec<u32>) {
    let Some(siz) = data.windows(4).position(|w| w == b"\xFF\x4F\xFF\x51") else {
        return (vec![], vec![]);
    };
    let siz = &data[siz + 4..];
    let u32_at = |i: usize| u32::from_be_bytes(siz[i..i + 4].try_into().unwrap());
    let (x_siz, y_siz, x_osiz, y_osiz) = (u32_at(4), u32_at(8), u32_at(12), u32_at(16));
    let (xt_siz, yt_siz, xto_siz, yto_siz) = (u32_at(20), u32_at(24), u32_at(28), u32_at(32));

    // Map reference grid positions to the output grid. This ignores rounding,
    // which is fine since any region is a valid test case.
    let boundaries = |offset: u32, step: u32, origin: u32, end: u32, size: u32| {
        let scale = f64::from(end - origin) / f64::from(size);
        (1..)
            .map(move |i| u64::from(offset) + i * u64::from(step))
            .take_while(move |b| *b < u64::from(end))
            .filter(move |b| *b > u64::from(origin))
            .map(move |b| ((b - u64::from(origin)) as f64 / scale) as u32)
            .filter(move |b| *b > 0 && *b < size)
            .take(4)
            .collect::<Vec<_>>()
    };

    (
        boundaries(xto_siz, xt_siz, x_osiz, x_siz, width),
        boundaries(yto_siz, yt_siz, y_osiz, y_siz, height),
    )
}

/// A set of regions covering corners, edges, single samples, odd sizes and
/// tile boundaries.
fn regions(width: u32, height: u32, tiles: &(Vec<u32>, Vec<u32>)) -> Vec<Region> {
    let region = |x: u32, y: u32, w: u32, h: u32| {
        let (x, y) = (x.min(width - 1), y.min(height - 1));
        Region {
            x,
            y,
            width: w.clamp(1, width - x),
            height: h.clamp(1, height - y),
        }
    };
    let (w, h) = (width, height);

    let mut regions = vec![
        // The whole image.
        region(0, 0, w, h),
        // Single samples in the corners and the center.
        region(0, 0, 1, 1),
        region(w.saturating_sub(1), 0, 1, 1),
        region(0, h.saturating_sub(1), 1, 1),
        region(w.saturating_sub(1), h.saturating_sub(1), 1, 1),
        region(w / 2, h / 2, 1, 1),
        // Corners.
        region(0, 0, w / 2 + 1, h / 3 + 1),
        region(w / 2, h / 2, w, h),
        region(w.saturating_sub(w / 3), 0, w, 7),
        region(0, h.saturating_sub(5), 9, h),
        // Edges.
        region(0, 0, w, 1),
        region(0, h.saturating_sub(1), w, 1),
        region(0, 0, 1, h),
        region(w.saturating_sub(1), 0, 1, h),
        region(1, 1, w.saturating_sub(2), h.saturating_sub(2)),
        // Odd sizes at odd positions.
        region((w / 3) | 1, (h / 5) | 1, 7, 5),
        region((w / 7) | 1, (h / 3) | 1, (w / 3) | 1, (h / 2) | 1),
        region(3, 5, 17, 33),
        region(w / 4, h / 4, w / 2, h / 2),
        region(13, 11, 1, 31),
        region(31, 1, 65, 1),
    ];

    // Regions straddling tile boundaries.
    for &x in &tiles.0 {
        regions.push(region(x.saturating_sub(3), h / 3, 6, 5));
        regions.push(region(x.saturating_sub(1), 0, 2, h));
    }
    for &y in &tiles.1 {
        regions.push(region(w / 3, y.saturating_sub(2), 5, 4));
        regions.push(region(0, y.saturating_sub(1), w, 2));
        for &x in &tiles.0 {
            regions.push(region(x.saturating_sub(1), y.saturating_sub(1), 2, 2));
            regions.push(region(x.saturating_sub(9), y.saturating_sub(9), 18, 18));
        }
    }

    regions
}

/// Decode `regions` of `stream` at every available resolution and compare
/// them with the crop of the full decode.
fn check_regions(stream: &str) -> usize {
    let data = read_stream(stream);
    let Ok(full) = decode_samples(&data, &DecodeSettings::default()) else {
        // Streams that are expected to fail are covered elsewhere.
        return 0;
    };

    let mut checked = 0;
    let mut previous_size = None;

    for reduction in 0..8 {
        let target_resolution = Some((
            (full.width >> reduction).max(1),
            (full.height >> reduction).max(1),
        ));
        let settings = DecodeSettings {
            target_resolution,
            ..Default::default()
        };
        let Ok(reference) = decode_samples(&data, &settings) else {
            break;
        };
        let size = (reference.width, reference.height);
        if previous_size == Some(size) {
            break;
        }
        previous_size = Some(size);

        let tiles = tile_boundaries(&data, size.0, size.1);

        for region in regions(size.0, size.1, &tiles) {
            let settings = DecodeSettings {
                region: Some(region),
                ..settings
            };
            let actual = decode_samples(&data, &settings).unwrap_or_else(|e| {
                panic!("{stream}: decoding {region:?} at reduction {reduction} failed: {e}")
            });
            compare_crop(stream, reduction, region, &reference, &actual);
            checked += 1;
        }
    }

    checked
}

fn compare_crop(
    stream: &str,
    reduction: u32,
    region: Region,
    reference: &Samples,
    actual: &Samples,
) {
    let context = format!("{stream} at reduction {reduction}, {region:?}");
    assert_eq!(
        (actual.width, actual.height),
        (region.width, region.height),
        "{context}: wrong dimensions"
    );
    assert_eq!(
        actual.components.len(),
        reference.components.len(),
        "{context}: wrong number of components"
    );

    let channels = reference.components.len();
    let (x0, y0) = (region.x as usize, region.y as usize);
    let (width, height) = (region.width as usize, region.height as usize);
    let reference_width = reference.width as usize;

    for y in 0..height {
        let reference_row = (y0 + y) * reference_width + x0;
        let actual_row = y * width;

        for (c, (expected, actual)) in reference
            .components
            .iter()
            .zip(&actual.components)
            .enumerate()
        {
            let expected = &expected[reference_row..][..width];
            let actual = &actual[actual_row..][..width];
            if let Some(x) = expected
                .iter()
                .zip(actual)
                .position(|(e, a)| e.to_bits() != a.to_bits())
            {
                panic!(
                    "{context}: component {c} differs at row {y}, column {x}: expected {}, got {}",
                    expected[x], actual[x]
                );
            }
        }

        assert_eq!(
            &reference.data_u8[reference_row * channels..][..width * channels],
            &actual.data_u8[actual_row * channels..][..width * channels],
            "{context}: 8-bit output differs in row {y}"
        );
    }
}

/// Every test stream, covering tiled, multi-component, subsampled,
/// irreversible 9-7 and reversible 5-3 images.
#[test]
fn all_streams_match_full_decode() {
    let mut streams = std::fs::read_dir(tests_dir().join("streams"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            path.file_stem().unwrap().to_str().unwrap().to_string()
        })
        .collect::<Vec<_>>();
    streams.sort();

    // Check the streams in parallel, this is by far the slowest test.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let checked: usize = std::thread::scope(|scope| {
        let workers = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut checked = 0;
                    while let Some(stream) =
                        streams.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                    {
                        checked += check_regions(stream);
                    }
                    checked
                })
            })
            .collect::<Vec<_>>();

        workers.into_iter().map(|w| w.join().unwrap()).sum()
    });
    assert!(checked > 10_000, "only {checked} regions were checked");
}

/// Make sure the streams with the properties that matter most for region
/// decoding are actually decodable and checked.
#[test]
fn covers_relevant_streams() {
    for stream in [
        // Tiled.
        "codestream_tiles_partial",
        "codestream_tiles_offset_many",
        "quantization_irreversible_tiles_offset",
        // Multi-component with MCT.
        "mct_rct",
        "mct_ict",
        // Subsampled.
        "codestream_subsampling_2x2",
        "codestream_subsampling_per_component",
        "codestream_subsampling_with_offset",
        "codestream_subsampling_odd_factors",
        // Irreversible 9-7 and reversible 5-3.
        "quantization_irreversible",
        "codestream_gray",
        "codestream_odd_size",
        "codestream_offset_large",
    ] {
        assert!(check_regions(stream) > 0, "{stream} was not checked");
    }
}

#[test]
fn invalid_regions() {
    let data = read_stream("codestream_gray");
    let image = Image::new(&data, &DecodeSettings::default()).unwrap();
    let (width, height) = (image.width(), image.height());

    for region in [
        Region {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Region {
            x: 0,
            y: 0,
            width: width + 1,
            height,
        },
        Region {
            x: width,
            y: 0,
            width: 1,
            height: 1,
        },
        Region {
            x: 0,
            y: 1,
            width: 1,
            height,
        },
        Region {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        let settings = DecodeSettings {
            region: Some(region),
            ..Default::default()
        };
        assert_eq!(
            Image::new(&data, &settings).err(),
            Some(DecodeError::Validation(ValidationError::InvalidRegion)),
            "{region:?}"
        );
    }
}
