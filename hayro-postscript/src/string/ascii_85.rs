// Keep in sync with `hayro-syntax/src/filter/ascii_85.rs`.

use crate::reader::{Reader, is_whitespace};
use alloc::vec::Vec;

pub(crate) fn decode_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    const POW_85: [u32; 5] = [52200625, 614125, 7225, 85, 1];

    let mut reader = Reader::new(data);

    let mut read_byte = || -> Option<u8> {
        loop {
            let b = reader.read_byte()?;

            // White space characters should be ignored.
            if !is_whitespace(b) {
                return Some(b);
            }
        }
    };

    let flush_group = |group: &mut Vec<u8>, decoded: &mut Vec<u8>| -> Option<()> {
        let (digits, output_len): ([u32; 5], usize) = match group.len() {
            0 => return Some(()),
            1 => return None, // A single character is not valid.
            2 => (
                [group[0], group[1], b'u', b'u', b'u'].map(|b| (b - b'!') as u32),
                1,
            ),
            3 => (
                [group[0], group[1], group[2], b'u', b'u'].map(|b| (b - b'!') as u32),
                2,
            ),
            4 => (
                [group[0], group[1], group[2], group[3], b'u'].map(|b| (b - b'!') as u32),
                3,
            ),
            5 => (
                [group[0], group[1], group[2], group[3], group[4]].map(|b| (b - b'!') as u32),
                4,
            ),
            _ => unreachable!(),
        };

        let value = digits[0]
            .checked_mul(POW_85[0])?
            .checked_add(digits[1].checked_mul(POW_85[1])?)?
            .checked_add(digits[2].checked_mul(POW_85[2])?)?
            .checked_add(digits[3].checked_mul(POW_85[3])?)?
            .checked_add(digits[4])?;

        decoded.extend_from_slice(&value.to_be_bytes()[..output_len]);
        group.clear();
        Some(())
    };

    out.reserve(data.len() * 4 / 5);
    let mut group = Vec::with_capacity(5);

    loop {
        let Some(b) = read_byte() else {
            // Be lenient and accept what we have (see PDFBOX-5910).
            flush_group(&mut group, out)?;

            return Some(());
        };

        match b {
            b'!'..=b'u' => {
                group.push(b);

                if group.len() == 5 {
                    flush_group(&mut group, out)?;
                }
            }
            b'z' => {
                flush_group(&mut group, out)?;
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
            b'~' => {
                // Technically requires a '>', but there is a PDF where it isn't
                // appended and decodes fine in other viewers.
                flush_group(&mut group, out)?;

                return Some(());
            }
            _ => return None, // Invalid character.
        }
    }
}
