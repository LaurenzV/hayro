use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

/// Decode until the output holds more than `limit` bytes, so that the caller
/// can tell whether the data decodes to more than `limit` bytes.
pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    while decoded.len() <= limit {
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
            decode(&input, usize::MAX).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }

    #[test]
    fn run_length_stops_past_limit() {
        // Each pair repeats a zero 128 times.
        let input = [129, 0].repeat(1000);
        let decoded = decode(&input, 10).unwrap();
        assert_eq!(decoded, vec![0; 128]);
    }
}
