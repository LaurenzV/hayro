//! This example shows you how to convert a JBIG2 image into a PNG file.

#![allow(missing_docs)]

use std::process::ExitCode;

use image::GrayImage;

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

    let image = match hayro_jbig2::decode(&data) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("Failed to decode JBIG2: {err}");

            return ExitCode::FAILURE;
        }
    };

    println!("Decoded: {}x{} image", image.width, image.height);

    struct LumaDecoder {
        buffer: Vec<u8>,
    }

    impl hayro_jbig2::Decoder for LumaDecoder {
        fn push_pixel(&mut self, black: bool) {
            self.buffer.push(if black { 0 } else { 255 });
        }

        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            let luma = if black { 0 } else { 255 };
            self.buffer
                .extend(std::iter::repeat_n(luma, chunk_count as usize * 8));
        }

        fn next_line(&mut self) {}
    }

    let mut decoder = LumaDecoder {
        buffer: Vec::with_capacity((image.width * image.height) as usize),
    };

    image.decode(&mut decoder);

    let Some(gray) = GrayImage::from_raw(image.width, image.height, decoder.buffer) else {
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
