use crate::reader::Reader;

pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    loop {
        let length = reader.read_byte()?;

        match length {
            128 => return Some(decoded),
            0..=127 => decoded.extend(reader.read_bytes(length as usize + 1)?),
            _ => {
                let length = 257 - length as usize;
                decoded.extend([reader.read_byte()?].repeat(length))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::filter::run_length::decode;

    #[test]
    fn run_length() {
        let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
        assert_eq!(
            decode(&input).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }
}
