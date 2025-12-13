use crate::bit::BitReader;
use crate::tables::{EOFB, Mode};
use log::warn;
use std::iter;

mod bit;
mod decode;
mod tables;

/// The encoding mode for CCITT fax decoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncodingMode {
    /// Group 4 (MMR) - Pure 2D encoding, no EOL codes.
    /// PDF K < 0.
    Group4,
    /// Group 3 1D (MH) - Pure 1D encoding with EOL codes.
    /// PDF K = 0.
    Group3_1D,
    /// Group 3 2D (MR) - Mixed 1D/2D encoding with EOL + tag bits.
    /// PDF K > 0. The value indicates that after each 1D reference line,
    /// at most K-1 lines may be 2D encoded.
    Group3_2D { k: u32 },
}

#[derive(Copy, Clone, Debug)]
pub struct DecodeSettings {
    pub strict: bool,
    pub columns: u32,
    pub rows: u32,
    pub end_of_block: bool,
    pub end_of_line: bool,
    pub rows_are_byte_aligned: bool,
    pub encoding: EncodingMode,
}

pub trait Decoder {
    /// Push a single packed byte. Each bit represents a pixel (1=white, 0=black).
    fn push_byte(&mut self, byte: u8);
    /// Push multiple copies of the same byte value (for efficient runs of same-color pixels).
    fn push_bytes(&mut self, byte: u8, count: usize);
    /// Called when a line is complete (after byte alignment).
    fn next_line(&mut self);
}

/// Accumulates individual bits into a byte buffer (MSB-first).
#[derive(Default)]
struct BitPacker {
    /// Accumulated bits (MSB-first).
    buffer: u8,
    /// Number of bits currently in the buffer (0-7).
    count: u8,
}

impl BitPacker {
    fn new() -> Self {
        Self::default()
    }

    /// Push a single bit. Returns `Some(byte)` if the buffer is now full.
    fn push_bit(&mut self, white: bool) -> Option<u8> {
        let bit = if white { 1 } else { 0 };
        self.buffer = (self.buffer << 1) | bit;
        self.count += 1;

        if self.count == 8 {
            let byte = self.buffer;
            self.buffer = 0;
            self.count = 0;
            Some(byte)
        } else {
            None
        }
    }

    /// Returns true if there are pending bits in the buffer.
    fn has_pending(&self) -> bool {
        self.count > 0
    }

    /// Flush any partial byte with zero padding. Returns `Some(byte)` if there were pending bits.
    fn flush(&mut self) -> Option<u8> {
        if self.count > 0 {
            let padded = self.buffer << (8 - self.count);
            self.buffer = 0;
            self.count = 0;
            Some(padded)
        } else {
            None
        }
    }
}

pub fn decode(data: &[u8], decoder: &mut impl Decoder, settings: &DecodeSettings) -> Option<usize> {
    let mut ctx = DecoderContext::new(decoder, settings);
    let mut reader = BitReader::new(data);

    match settings.encoding {
        EncodingMode::Group4 => decode_group4(&mut ctx, &mut reader)?,
        EncodingMode::Group3_1D => decode_group3_1d(&mut ctx, &mut reader)?,
        EncodingMode::Group3_2D { .. } => {
            unimplemented!();
        }
    }

    reader.align();
    Some(reader.byte_pos())
}

fn decode_group3_1d<T: Decoder>(ctx: &mut DecoderContext<T>, reader: &mut BitReader) -> Option<()> {
    // It seems like PDF producers are a bit sloppy with the `end_of_line` flag,
    // so we just always try to read one.
    let _ = reader.read_eol_if_available();

    loop {
        while ctx.a0().unwrap_or(0) < ctx.max_idx {
            let run_length = reader.decode_run(ctx.is_white)? as usize;
            ctx.push_pixels(run_length);
            ctx.is_white = !ctx.is_white;
        }

        ctx.check_eol(reader)?;

        let num_eol = reader.read_eol_if_available();

        if num_eol == 6 {
            break;
        }
    }

    Some(())
}

fn decode_group4<T: Decoder>(ctx: &mut DecoderContext<T>, reader: &mut BitReader) -> Option<()> {
    loop {
        if ctx.settings.end_of_block {
            // In this case, bit stream is terminated by an explicit marker.
            if reader.peak_bits(24) == Some(EOFB) {
                // Consume the EOFB marker
                reader.read_bits(24);
                break;
            }
        } else {
            // Otherwise, the length needs to be inferred from the number of
            // expected rows.
            if ctx.decoded_rows == ctx.settings.rows {
                break;
            }
        }

        let mode = reader.decode_mode()?;

        match mode {
            // 2.2.3.1 Pass mode.
            Mode::Pass => {
                ctx.push_pixels(ctx.b2 - ctx.a0().unwrap_or(0));
                ctx.start_run();
                // No color change happens in pass mode.
            }
            // 2.2.3.3 Horizontal mode.
            Mode::Horizontal => {
                let a0a1 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a0a1);
                ctx.is_white = !ctx.is_white;

                let a1a2 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a1a2);
                ctx.is_white = !ctx.is_white;

                ctx.check_eol(reader)?;
            }
            // 2.2.3.2 Vertical mode.
            Mode::Vertical(i) => {
                let a1 = if i >= 0 {
                    ctx.b1.checked_add(i as usize)?
                } else {
                    ctx.b1.checked_sub((-i) as usize)?
                };

                let a0 = ctx.a0().unwrap_or(0);

                ctx.push_pixels(a1.checked_sub(a0)?);
                ctx.is_white = !ctx.is_white;

                ctx.check_eol(reader)?;
            }
        }
    }

    Some(())
}

struct DecoderContext<'a, T: Decoder> {
    /// The previous line.
    reference_line: Vec<u8>,
    /// The line we are currently decoding.
    coding_line: Vec<u8>,
    /// The decoder sink.
    decoder: &'a mut T,
    /// Packs bits into bytes.
    packer: BitPacker,
    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    b1: usize,
    /// "The next changing element to the right of b1, on the reference line."
    b2: usize,
    /// The maximum permissible index for all variables.
    max_idx: usize,
    /// Whether the next run to be decoded is white.
    is_white: bool,
    /// How many rows have been decoded so far.
    decoded_rows: u32,
    settings: &'a DecodeSettings,
}

impl<'a, T: Decoder> DecoderContext<'a, T> {
    fn new(decoder: &'a mut T, settings: &'a DecodeSettings) -> DecoderContext<'a, T> {
        // We add a padding of one on the right so that when any of the pointers
        // has reached the maximum index (which is exactly settings.column), we
        // don't get an OOB access when accessing the field in `find_b1`/`find_b2`.
        let max_idx = settings.columns as usize;
        let len = max_idx + 1;

        Self {
            // "The reference line for the first coding line in a
            // page is an imaginary white line."
            reference_line: vec![0; len],
            coding_line: vec![],
            decoder,
            packer: BitPacker::new(),
            b1: max_idx,
            b2: max_idx,
            max_idx,
            is_white: true,
            decoded_rows: 0,
            settings,
        }
    }

    /// `a0` refers to the first changing element on the current line.
    fn a0(&self) -> Option<usize> {
        if self.coding_line.is_empty() {
            // If we haven't coded anything yet, a0 conceptually points at the
            // index -1. This is a bit of an edge case, and we therefore require
            // callers of this method to handle the case themselves.
            None
        } else {
            // Otherwise, the index just point to the next element to be decoded.
            Some(self.coding_line.len())
        }
    }

    fn find_b1(&mut self) {
        // b1 refers to an element of the opposite color.
        let target_color = self.cur_color() ^ 1;

        // If we have an a0, b1 must start at the RIGHT of that element. Otherwise,
        // it starts from the first possible index (0), and the last color is the
        // imaginary white element on the left.
        let (start, mut last_color) = if let Some(a0) = self.a0() {
            (a0 + 1, self.reference_line[a0])
        } else {
            (0, 0)
        };

        self.b1 = start;

        while self.b1 < self.max_idx {
            let current_color = self.reference_line[self.b1];

            if current_color != last_color && current_color == target_color {
                break;
            }

            last_color = current_color;
            self.b1 += 1;
        }
    }

    fn find_b2(&mut self) {
        self.b2 = self.b1;

        let b1_color = self.reference_line[self.b1];

        while self.b2 < self.max_idx {
            if self.reference_line[self.b2] != b1_color {
                break;
            }

            self.b2 += 1;
        }
    }

    fn push_pixels(&mut self, count: usize) {
        let white = self.is_white;
        let byte_val: u8 = if white { 0xFF } else { 0x00 };
        let mut remaining = count;

        // Fill partial byte buffer to boundary.
        while self.packer.has_pending() && remaining > 0 {
            if let Some(byte) = self.packer.push_bit(white) {
                self.decoder.push_byte(byte);
            }
            remaining -= 1;
        }

        // Push full bytes.
        let full_bytes = remaining / 8;
        if full_bytes > 0 {
            self.decoder.push_bytes(byte_val, full_bytes);
            remaining %= 8;
        }

        // Push remaining bits into buffer.
        for _ in 0..remaining {
            if let Some(byte) = self.packer.push_bit(white) {
                self.decoder.push_byte(byte);
            }
        }

        // Also push the pixels to the coding line.
        let cur_color = self.cur_color();
        self.coding_line.extend(iter::repeat_n(cur_color, count));
    }

    fn cur_color(&self) -> u8 {
        if self.is_white { 0 } else { 1 }
    }

    fn start_run(&mut self) {
        self.find_b1();
        self.find_b2();
    }

    fn check_eol(&mut self, reader: &mut BitReader) -> Option<()> {
        if self.a0().unwrap_or(0) >= self.max_idx {
            // Go to next line.

            if self.coding_line.len() != self.settings.columns as usize {
                warn!("coding line has wrong size");

                return None;
            }

            // Flush any partial byte with zero padding before finishing the line.
            if let Some(byte) = self.packer.flush() {
                self.decoder.push_byte(byte);
            }

            core::mem::swap(&mut self.reference_line, &mut self.coding_line);
            self.reference_line.resize(self.max_idx + 1, 0);
            self.coding_line.clear();
            self.is_white = true;
            self.decoded_rows += 1;
            self.decoder.next_line();

            if self.settings.rows_are_byte_aligned {
                reader.align();
            }
        }

        self.start_run();

        Some(())
    }
}
