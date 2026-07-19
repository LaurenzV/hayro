use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

pub(crate) fn decode(data: &[u8], max_decoded: Option<usize>) -> Option<Vec<u8>> {
    let max_decoded = max_decoded.unwrap_or(usize::MAX);
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    loop {
        // Bound the output against expansion bombs: each 2-byte repeat pair
        // can append up to 128 bytes, and `/Filter` arrays can chain layers.
        // Checked once per iteration, so the peak overshoot is one run.
        if decoded.len() > max_decoded {
            debug!("run-length stream exceeds the decoded-size limit");
            return None;
        }

        let length = reader.read_byte()?;

        match length {
            128 => break,
            0..=127 => {
                // PDFBOX-3990, just abort early if stream is invalid.
                let Some(bytes) = reader.read_bytes(length as usize + 1) else {
                    break;
                };

                decoded.extend(bytes);
            }
            _ => {
                let length = 257 - length as usize;
                decoded.extend([reader.read_byte()?].repeat(length));
            }
        }
    }

    Some(decoded)
}

#[cfg(test)]
mod tests {
    use crate::filter::run_length::decode;

    #[test]
    fn run_length() {
        let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
        assert_eq!(
            decode(&input, None).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }

    #[test]
    fn run_length_rejects_expansion_bomb() {
        const LIMIT: usize = 64 * 1024;

        // Each [129, 0] pair expands to 128 zero bytes (257 - 129), so this
        // ~1 KiB input expands past the limit and must be refused rather
        // than ballooned. Without a limit it decodes fine, proving the
        // rejection comes from the limit and not from a malformed stream.
        let pairs = LIMIT / 128 + 2;
        let mut input = Vec::with_capacity(pairs * 2 + 1);
        for _ in 0..pairs {
            input.extend_from_slice(&[129, 0]);
        }
        input.push(128); // EOD
        assert_eq!(decode(&input, Some(LIMIT)), None);
        assert!(decode(&input, None).is_some());
    }
}
