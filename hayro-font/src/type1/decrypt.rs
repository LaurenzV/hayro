use super::stream::Stream;
use log::error;

pub(crate) fn decrypt(data: &[u8]) -> Option<Vec<u8>> {
    let mut stream = Stream::new(data);
    stream.skip_whitespaces();

    let mut b00 = None;
    let mut r: u32 = 55665;

    let mut decrypt = |b: u8| decrypt_byte(b, &mut r);

    for _ in 0..1000 {
        let c = stream.read_byte()?;
        if !is_white_space_after_token_eexec(c) {
            b00 = Some(c);
            break;
        }
    }

    let Some(b00) = b00 else {
        error!("b00 was None");

        return None;
    };

    let mut b = [0u8; 4];
    b[0] = b00;

    for i in 1..=3 {
        let c = stream.read_byte()?;
        b[i] = c;
    }

    let mut is_bin = false;

    for i in 0..4 {
        if !b[i].is_ascii_hexdigit() {
            is_bin = true;
        }
    }

    if is_bin {
        let mut out = vec![];

        for i in 0..4 {
            decrypt(b[i]);
        }

        for b in stream.tail()? {
            out.push(decrypt(*b));
        }

        Some(out)
    } else {
        let mut out = vec![];

        // Decrypt the first 4 hex chars (2 bytes) as garbage bytes
        let b0 = hex_to_byte(b[0] as char, b[1] as char)?;
        let b1 = hex_to_byte(b[2] as char, b[3] as char)?;
        decrypt(b0);
        decrypt(b1);

        let mut hex_chars = vec![];
        for b in stream.tail()? {
            if b.is_ascii_hexdigit() {
                hex_chars.push(*b as char);
            } else if !is_whitespace(*b) {
                break;
            }
        }

        let mut i = 0;
        while i + 1 < hex_chars.len() {
            if let Some(byte) = hex_to_byte(hex_chars[i], hex_chars[i + 1]) {
                if i < 4 {
                    decrypt(byte);
                } else {
                    out.push(decrypt(byte));
                }
                i += 2;
            } else {
                error!("failed to convert hex chars to byte");
                return None;
            }
        }

        // Handle odd number of hex digits (pad with '0')
        if i < hex_chars.len() {
            if let Some(byte) = hex_to_byte(hex_chars[i], '0') {
                if i >= 4 {
                    out.push(decrypt(byte));
                } else {
                    decrypt(byte);
                }
            }
        }

        Some(out)
    }
}

pub(crate) fn decrypt_byte(cipher: u8, r: &mut u32) -> u8 {
    let cipher = cipher as u32;
    let plain = cipher ^ (*r >> 8);
    *r = ((cipher + *r).wrapping_mul(52845) + 22719) & 0xFFFF;
    (plain & 0xFF) as u8
}

fn is_white_space_after_token_eexec(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | b'\x0C')
}

fn hex_to_byte(c1: char, c2: char) -> Option<u8> {
    let h1 = hex_to_dec(c1)?;
    let h2 = hex_to_dec(c2)?;
    Some((h1 << 4) | h2)
}

fn hex_to_dec(hex: char) -> Option<u8> {
    match hex {
        '0'..='9' => Some((hex as u8) - b'0'),
        'A'..='F' => Some((hex as u8) - b'A' + 10),
        'a'..='f' => Some((hex as u8) - b'a' + 10),
        _ => None,
    }
}
