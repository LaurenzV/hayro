#![no_main]

use hayro_jpeg2000::{DecodeSettings, Image};
use image::ImageDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let settings = DecodeSettings::default();
    if let Ok(image) = Image::new(data, &settings) {
        // Let's ignore larger images so we don't time out.
        if image.width() <= 2500 && image.height() <= 2500 {
            let mut buf = vec![0_u8; image.total_bytes() as usize];
            let _ = image.read_image(&mut buf);
        }
    }
});
