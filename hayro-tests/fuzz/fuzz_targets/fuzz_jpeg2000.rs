#![no_main]

use libfuzzer_sys::fuzz_target;
use hayro_jpeg2000::{Image, DecodeSettings};
use image::ImageDecoder;

fuzz_target!(|data: &[u8]| {
    let settings = DecodeSettings::default();
    if let Ok(image) = Image::new(&data, &settings) {
        let mut buf = vec![0_u8; image.total_bytes() as usize];
        let _ = image.read_image(&mut buf);
    }
});
