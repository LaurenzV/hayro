use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::Scanner;

pub(crate) trait ScannerExt {
    fn read_string(&mut self, buf: &mut Vec<u8>) -> Option<String>;
    fn read_integer(&mut self) -> Option<i32>;
    fn read_u32_code(&mut self, buf: &mut Vec<u8>) -> Option<u32>;
}

impl ScannerExt for Scanner<'_> {
    fn read_string(&mut self, buf: &mut Vec<u8>) -> Option<String> {
        let s = self.parse_string().ok()?;
        s.decode_into(buf).ok()?;
        String::from_utf8(buf.to_vec()).ok()
    }

    fn read_integer(&mut self) -> Option<i32> {
        let n = self.parse_number().ok()?;
        Some(n.as_i32())
    }

    fn read_u32_code(&mut self, buf: &mut Vec<u8>) -> Option<u32> {
        let s = self.parse_string().ok()?;
        s.decode_into(buf).ok()?;
        if buf.len() > 4 {
            return None;
        }
        let mut val = 0u32;
        for &b in buf.iter() {
            val = (val << 8) | u32::from(b);
        }
        Some(val)
    }
}
