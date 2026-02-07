use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::Scanner;

use crate::CharacterCode;

pub(crate) trait ScannerExt {
    fn read_string(&mut self, buf: &mut Vec<u8>) -> Option<String>;
    fn read_integer(&mut self) -> Option<i32>;
    fn read_char_code(&mut self, buf: &mut Vec<u8>) -> Option<CharacterCode>;
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

    fn read_char_code(&mut self, buf: &mut Vec<u8>) -> Option<CharacterCode> {
        let s = self.parse_string().ok()?;
        s.decode_into(buf).ok()?;
        Some(CharacterCode::from_bytes(buf))
    }
}
