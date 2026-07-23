use super::Pace;
use crate::object::stream::DecodeFailure;
use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;
use enough::Stop;

pub(crate) fn decode(data: &[u8], stop: &dyn Stop) -> Result<Vec<u8>, DecodeFailure> {
    let stop = stop.may_stop().then_some(stop);
    let mut pace = Pace::new(64 * 1024);
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    loop {
        pace.poll(decoded.len(), stop)?;
        let length = reader.read_byte().ok_or(DecodeFailure::StreamDecode)?;

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
                let byte = reader.read_byte().ok_or(DecodeFailure::StreamDecode)?;
                decoded.extend([byte].repeat(length));
            }
        }
    }

    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use crate::filter::run_length::decode;

    #[test]
    fn run_length() {
        let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
        assert_eq!(
            decode(&input, &enough::Unstoppable).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }
}
