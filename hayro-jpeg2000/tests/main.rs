//! Each test decodes a JPEG 2000 file from `streams` and compares the decoded
//! samples against an image in `references`.

mod code_blocks;
mod codestream;
mod errors;
mod integration;
mod jp2;
mod mct;
mod packets;
mod palette;
mod quantization;
mod roi;
mod tile_parts;
mod wavelet;

use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use image::ImageDecoder;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

struct Reference {
    width: u32,
    height: u32,
    channels: usize,
    samples: Vec<u8>,
    tolerance: u8,
}

fn reference(name: &str) -> Reference {
    let path = tests_dir().join("references").join(format!("{name}.png"));
    let file =
        File::open(&path).unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display()));
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()));
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()));
    assert_eq!(
        frame.bit_depth,
        png::BitDepth::Eight,
        "{}: references are 8-bit",
        path.display()
    );

    Reference {
        width: frame.width,
        height: frame.height,
        channels: frame.color_type.samples(),
        samples: buf,
        tolerance: 0,
    }
}

impl Reference {
    fn tolerance(mut self, tolerance: u8) -> Self {
        self.tolerance = tolerance;
        self
    }

    fn compare(&self, stream: &str, actual: &Decoded) {
        let Decoded {
            width,
            height,
            channels,
            ref samples,
        } = *actual;
        assert_eq!(
            (width, height),
            (self.width, self.height),
            "{stream}: wrong dimensions"
        );
        assert_eq!(
            channels, self.channels,
            "{stream}: wrong number of components"
        );

        let differences = samples
            .iter()
            .zip(&self.samples)
            .map(|(a, e)| a.abs_diff(*e));
        if let Some(i) = differences.clone().position(|d| d > self.tolerance) {
            let pixel = i / channels;
            panic!(
                "{stream}: first mismatch in component {} at row {}, column {}: expected {}, got {} (largest difference {})",
                i % channels,
                pixel / width as usize,
                pixel % width as usize,
                self.samples[i],
                samples[i],
                differences.max().unwrap()
            );
        }
    }
}

fn stream_path(stream: &str) -> PathBuf {
    ["j2k", "jp2"]
        .iter()
        .map(|ext| tests_dir().join("streams").join(format!("{stream}.{ext}")))
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("no stream named {stream}"))
}

struct Decoded {
    width: u32,
    height: u32,
    channels: usize,
    samples: Vec<u8>,
}

fn read_stream(stream: &str) -> Vec<u8> {
    let path = stream_path(stream);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn decode(stream: &str, settings: &DecodeSettings) -> Result<Decoded, String> {
    let data = read_stream(stream);
    let image = Image::new(&data, settings).map_err(|e| e.to_string())?;
    let (width, height) = (image.width(), image.height());
    let mut ctx = DecoderContext::default();
    let decoded = image.decode(&mut ctx).map_err(|e| e.to_string())?;

    Ok(Decoded {
        width,
        height,
        channels: decoded.components().len(),
        samples: decoded.data_u8(),
    })
}

fn decode_output(stream: &str) -> Result<Decoded, String> {
    let data = read_stream(stream);
    let image = Image::new(&data, &DecodeSettings::default()).map_err(|e| e.to_string())?;
    let (width, height) = image.dimensions();
    let channels = usize::from(image.color_type().channel_count());
    let mut samples = vec![0; image.total_bytes() as usize];
    image.read_image(&mut samples).map_err(|e| e.to_string())?;

    Ok(Decoded {
        width,
        height,
        channels,
        samples,
    })
}

fn check(stream: &str, expected: Reference) {
    check_with(stream, DecodeSettings::default(), expected);
}

fn check_with(stream: &str, settings: DecodeSettings, expected: Reference) {
    let actual =
        decode(stream, &settings).unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));
    expected.compare(stream, &actual);
}

fn check_components(stream: &str, expected: Reference) {
    let actual = decode(stream, &DecodeSettings::default())
        .unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));
    let (width, channels) = (actual.width as usize, actual.channels);
    let samples = actual
        .samples
        .chunks_exact(width * channels)
        .flat_map(|row| (0..channels).flat_map(move |c| row.iter().skip(c).step_by(channels)))
        .copied()
        .collect();

    let side_by_side = Decoded {
        width: actual.width * channels as u32,
        height: actual.height,
        channels: 1,
        samples,
    };
    expected.compare(stream, &side_by_side);
}

fn check_output(stream: &str, expected: Reference) {
    let actual = decode_output(stream).unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));
    expected.compare(stream, &actual);
}

fn check_error(stream: &str, settings: DecodeSettings, message: &str) {
    match decode(stream, &settings) {
        Ok(_) => panic!("{stream}: decoding succeeded, but should have failed"),
        Err(e) => assert!(
            e.contains(message),
            "{stream}: expected an error containing \"{message}\", got \"{e}\""
        ),
    }
}
