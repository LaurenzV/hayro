use std::iter;

mod bit;

pub struct DecodeSettings {
    pub strict: bool,
    pub columns: u32,
    pub rows: u32,
    pub eoblock: bool,
}

pub trait Decoder {
    fn push_pixels(&mut self, count: u16, black: bool);
    fn end_of_line(&mut self);
}

struct DecoderContext<'a, T: Decoder> {
    /// The previous line.
    reference_line: Vec<u8>,
    /// The line we are currently decoding.
    coding_line: Vec<u8>,
    /// The decoder sink.
    decoder: &'a mut T,
    /// "The reference or starting changing element on the coding line."
    a0: usize,
    /// "The next changing element to the right of a0, on the coding line."
    a1: usize,
    /// "The next changing element to the right of a1, on the coding line."
    a2: usize,
    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    b1: usize,
    /// "The next changing element to the right of b1, on the reference line."
    b2: usize,
}

impl<'a, T: Decoder> DecoderContext<'a, T> {
    fn new(decoder: &mut T, columns: u32) -> DecoderContext<'a, T> {
        // +1 so that we can emulate the imaginary first white element.
        let total_len = columns as usize + 1;

        Self {
            // "The reference line for the first coding line in a
            // page is an imaginary white line."
            reference_line: vec![0; total_len],
            coding_line: vec![0; total_len],
            decoder,
            a0: 0,
            a1: total_len,
            a2: total_len,
            b1: total_len,
            b2: total_len,
        }
    }

    fn push_pixels(&mut self, count: u16, black: bool) {
        self.decoder.push_pixels(count, black);
        let val = if black { 1 } else { 0 };
        self.coding_line.extend(iter::repeat_n(val, count as usize))
    }
}
