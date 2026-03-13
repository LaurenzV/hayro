//! This example shows you how to convert a JBIG2 image into a PNG file.

#![allow(missing_docs)]

use std::process::ExitCode;

use image::{GrayImage, ImageDecoder};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: {} <input.jbig2> <output.png>", args[0]);

        return ExitCode::FAILURE;
    }

    let input_path = &args[1];
    let output_path = &args[2];

    let data = match std::fs::read(input_path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Failed to read input file: {err}");

            return ExitCode::FAILURE;
        }
    };

    let image = match hayro_jbig2::Image::new(&data) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("Failed to parse JBIG2: {err}");

            return ExitCode::FAILURE;
        }
    };

    let (width, height) = image.dimensions();
    println!("Decoded: {width}x{height} image");

    let mut buf = vec![0_u8; image.total_bytes() as usize];
    if let Err(err) = image.read_image(&mut buf) {
        eprintln!("Failed to decode JBIG2: {err}");

        return ExitCode::FAILURE;
    }

    let Some(gray) = GrayImage::from_raw(width, height, buf) else {
        eprintln!("Internal error: Buffer size mismatch");

        return ExitCode::FAILURE;
    };

    if let Err(err) = gray.save(output_path) {
        eprintln!("Failed to save PNG: {err}");

        return ExitCode::FAILURE;
    }

    eprintln!("Saved: {output_path}");

    ExitCode::SUCCESS
}
