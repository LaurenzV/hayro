use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;
use enough::Stop;

pub(crate) fn decode(data: &[u8], stop: &dyn Stop) -> Option<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut decoded = vec![];
    const STOP_CHUNK: usize = 64 * 1024;
    let mut next_stop_check = STOP_CHUNK;

    loop {
        // Poll the stop check about every STOP_CHUNK output bytes.
        if decoded.len() >= next_stop_check {
            if stop.should_stop() {
                return None;
            }
            next_stop_check = decoded.len() + STOP_CHUNK;
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
            decode(&input, &enough::Unstoppable).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }
}
