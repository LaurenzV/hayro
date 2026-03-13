#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(image) = hayro_jbig2::Image::new(data) {
        struct NullDecoder;
        impl hayro_jbig2::Decoder for NullDecoder {
            fn push_pixel(&mut self, _black: bool) {}
            fn push_pixel_chunk(&mut self, _black: bool, _chunk_count: u32) {}
            fn next_line(&mut self) {}
        }
        let _ = image.decode(&mut NullDecoder);
    }
});
