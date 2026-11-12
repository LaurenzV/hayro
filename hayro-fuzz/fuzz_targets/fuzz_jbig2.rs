#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(file) = hayro_jbig2::File::new(data, &hayro_jbig2::DecodeSettings::default()) {
        for page in file.pages() {
            let _ = page.decode();
        }
    }
});
