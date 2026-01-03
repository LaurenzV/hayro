//! A decoder for CCITT fax-encoded images.
//!
//! This crate implements the CCITT Group 3 and Group 4 fax compression algorithms
//! as defined in ITU-T Recommendations T.4 and T.6. These encodings are commonly
//! used for bi-level (black and white) images in PDF documents and fax transmissions.
//!
//! The main entry point is the [decode] function, which takes encoded data and
//! decoding settings, and outputs the decoded pixels through a [Decoder] trait.
//!
//! The crate is `no_std` compatible but requires an allocator to be available.
//!
//! # Safety
//! Unsafe code is forbidden via a crate-level attribute.

#![no_std]
#![forbid(unsafe_code)]
#![forbid(missing_docs)]

extern crate alloc;

use crate::bit::BitReader;
use crate::states::{EOFB, Mode};

use alloc::vec;
use alloc::vec::Vec;

mod bit;
mod decode;
mod states;

/// A specialized Result type for CCITT decoding operations.
pub type Result<T> = core::result::Result<T, DecodeError>;

/// An error that can occur during CCITT decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Unexpected end of input while reading bits.
    UnexpectedEof,
    /// Invalid Huffman code sequence was encountered during decoding.
    InvalidCode,
    /// A scanline didn't have the expected number of pixels.
    LineLengthMismatch,
    /// Arithmetic overflow in run length or position calculation.
    Overflow,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of input"),
            Self::InvalidCode => write!(f, "invalid CCITT code sequence"),
            Self::LineLengthMismatch => write!(f, "scanline length mismatch"),
            Self::Overflow => write!(f, "arithmetic overflow in position calculation"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// The encoding mode for CCITT fax decoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncodingMode {
    /// Group 4 (MMR).
    Group4,
    /// Group 3 1D (MH).
    Group3_1D,
    /// Group 3 2D (MR).
    Group3_2D {
        /// The K parameter.
        k: u32,
    },
}

/// Settings to apply during decoding.
#[derive(Copy, Clone, Debug)]
pub struct DecodeSettings {
    /// How many columns the image has (i.e. its width).
    pub columns: u32,
    /// How many rows the image has (i.e. its height).
    ///
    /// In case `end_of_block` has been set to true, decoding will run until
    /// the given number of rows have been decoded, or the `end_of_block` marker
    /// has been encountered, whichever occurs first.
    pub rows: u32,
    /// Whether the stream _MAY_ contain an end-of-block marker
    /// (It doesn't have to. In that case this is set to `true` but there are
    /// no end-of-block markers, hayro-ccitt will still use the value of `rows`
    /// to determine when to stop decoding).
    pub end_of_block: bool,
    /// Whether the stream contains end-of-line markers.
    pub end_of_line: bool,
    /// Whether the data in the stream for each row is aligned to the byte
    /// boundary.
    pub rows_are_byte_aligned: bool,
    /// The encoding mode used by the image.
    pub encoding: EncodingMode,
    /// Whether black and white should be inverted.
    pub invert_black: bool,
}

/// A decoder for CCITT images.
pub trait Decoder {
    /// Push a single packed byte containing the data for 8 pixels.
    /// Each bit represents one pixel (1 for white and 0 for black).
    fn push_byte(&mut self, byte: u8);
    /// Push multiple columns of same-color pixels. The `byte` value will either
    /// be 0xFF if all pixels are white or 0x00 if all pixels are black.
    ///
    /// The `count` parameter indicates how many such bytes such be pushed.
    /// For example, if that method is called with `byte = 0xFF` and
    /// `count = 10`, we have 80 white pixels in total.
    fn push_bytes(&mut self, byte: u8, count: usize);
    /// Called when a row has been completed.
    fn next_line(&mut self);
}

/// Represents a color change at a specific index in a line.
#[derive(Clone, Copy)]
struct ColorChange {
    idx: usize,
    color: u8,
}

/// Accumulates individual bits into a byte buffer.
#[derive(Default)]
struct BitPacker {
    /// Accumulated bits.
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

/// Decode the given image using the provided settings and the decoder.
///
/// If decoding was successful, the number of bytes that have been read in total
/// is returned.
///
/// If an error is returned, it means that the file is somehow malformed.
/// However, even if that's the case, it is possible that a number
/// of rows were decoded successfully and written into the decoder, so those
/// can still be used, but the image might be truncated.
pub fn decode(data: &[u8], decoder: &mut impl Decoder, settings: &DecodeSettings) -> Result<usize> {
    let mut ctx = DecoderContext::new(decoder, settings);
    let mut reader = BitReader::new(data);

    match settings.encoding {
        EncodingMode::Group4 => decode_group4(&mut ctx, &mut reader)?,
        EncodingMode::Group3_1D => decode_group3_1d(&mut ctx, &mut reader)?,
        EncodingMode::Group3_2D { .. } => decode_group3_2d(&mut ctx, &mut reader)?,
    }

    reader.align();
    Ok(reader.byte_pos())
}

fn decode_group3_1d<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    // It seems like PDF producers are a bit sloppy with the `end_of_line` flag,
    // so we just always try to read one.
    let _ = reader.read_eol_if_available();

    loop {
        decode_1d_line(ctx, reader)?;
        ctx.next_line(reader)?;

        if group3_check_eob(ctx, reader) {
            break;
        }
    }

    Ok(())
}

fn decode_group3_2d<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    // It seems like PDF producers are a bit sloppy with the `end_of_line` flag,
    // so we just always try to read one.
    let _ = reader.read_eol_if_available();

    loop {
        let tag_bit = reader.read_bit()?;

        if tag_bit == 1 {
            decode_1d_line(ctx, reader)?;
        } else {
            decode_2d_line(ctx, reader)?;
        }

        ctx.next_line(reader)?;

        if group3_check_eob(ctx, reader) {
            break;
        }
    }

    Ok(())
}

fn group3_check_eob<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> bool {
    let num_eol = reader.read_eol_if_available();

    // PDFBOX-2778 has 7 EOL, although it should only be 6. Let's be lenient
    // and check with >=.
    if ctx.settings.end_of_block && num_eol >= 6 {
        // RTC (Return To Control).
        return true;
    }

    if ctx.decoded_rows == ctx.settings.rows || reader.at_end() {
        return true;
    }

    false
}

fn decode_group4<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    loop {
        if ctx.settings.end_of_block && reader.peak_bits(24) == Ok(EOFB) {
            reader.read_bits(24)?;
            break;
        }

        if ctx.decoded_rows == ctx.settings.rows || reader.at_end() {
            break;
        }

        decode_2d_line(ctx, reader)?;
        ctx.next_line(reader)?;
    }

    Ok(())
}

#[inline(always)]
fn decode_1d_line<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    while !ctx.at_eol() {
        let run_length = reader.decode_run(ctx.is_white)? as usize;
        ctx.push_pixels(run_length);
        ctx.is_white = !ctx.is_white;
    }

    Ok(())
}

#[inline(always)]
fn decode_2d_line<T: Decoder>(
    ctx: &mut DecoderContext<'_, T>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    while !ctx.at_eol() {
        let mode = reader.decode_mode()?;

        match mode {
            // 2.2.3.1 Pass mode.
            Mode::Pass => {
                ctx.push_pixels(ctx.b2() - ctx.a0().unwrap_or(0));
                ctx.update_b();
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

                ctx.update_b();
            }
            // 2.2.3.2 Vertical mode.
            Mode::Vertical(i) => {
                let b1 = ctx.b1();
                let a1 = if i >= 0 {
                    b1.checked_add(i as usize).ok_or(DecodeError::Overflow)?
                } else {
                    b1.checked_sub((-i) as usize).ok_or(DecodeError::Overflow)?
                };

                let a0 = ctx.a0().unwrap_or(0);

                ctx.push_pixels(a1.checked_sub(a0).ok_or(DecodeError::Overflow)?);
                ctx.is_white = !ctx.is_white;

                ctx.update_b();
            }
        }
    }

    Ok(())
}

struct DecoderContext<'a, T: Decoder> {
    /// Color changes in the reference line (previous line).
    ref_changes: Vec<ColorChange>,
    /// The minimum index we need to start from when searching for b1.
    ref_pos: usize,
    /// The current index of b1.
    b1_idx: usize,
    /// Color changes in the coding line (current line being decoded).
    coding_changes: Vec<ColorChange>,
    /// Current length of the coding line in pixels.
    coding_line_len: usize,
    /// The decoder sink.
    decoder: &'a mut T,
    /// The current byte we are writing.
    packer: BitPacker,
    /// The maximum permissible index for all "pointer" variables (i.e. a0, b1 and b2).
    max_idx: usize,
    /// Whether the next run to be decoded is white.
    is_white: bool,
    /// How many rows have been decoded so far.
    decoded_rows: u32,
    /// The settings to apply during decoding.
    settings: &'a DecodeSettings,
    /// Precomputed mask for inverting output bytes if the `invert_black` option
    /// has been set to `true`.
    invert_mask: u8,
}

impl<'a, T: Decoder> DecoderContext<'a, T> {
    fn new(decoder: &'a mut T, settings: &'a DecodeSettings) -> Self {
        let max_idx = settings.columns as usize;

        Self {
            ref_changes: vec![],
            ref_pos: 0,
            b1_idx: 0,
            coding_changes: Vec::new(),
            coding_line_len: 0,
            decoder,
            packer: BitPacker::new(),
            max_idx,
            // Each run starts with a white color.
            is_white: true,
            decoded_rows: 0,
            settings,
            invert_mask: if settings.invert_black { 0xFF } else { 0x00 },
        }
    }

    /// `a0` refers to the first changing element on the current line.
    fn a0(&self) -> Option<usize> {
        if self.coding_line_len == 0 {
            // If we haven't coded anything yet, a0 conceptually points at the
            // index -1. This is a bit of an edge case, and we therefore require
            // callers of this method to handle the case themselves.
            None
        } else {
            // Otherwise, the index points to the next element to be decoded.
            Some(self.coding_line_len)
        }
    }

    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    fn b1(&self) -> usize {
        self.ref_changes
            .get(self.b1_idx)
            .map_or(self.max_idx, |c| c.idx)
    }

    /// "The next changing element to the right of b1, on the reference line."
    fn b2(&self) -> usize {
        self.ref_changes
            .get(self.b1_idx + 1)
            .map_or(self.max_idx, |c| c.idx)
    }

    /// Compute the new position of b1 (and implicitly b2).
    #[inline(always)]
    fn update_b(&mut self) {
        // b1 refers to an element of the opposite color.
        let target_color = self.cur_color() ^ 1;
        // b1 must be strictly greater than a0.
        let min_idx = self.a0().map_or(0, |a| a + 1);

        self.b1_idx = self.max_idx;

        for i in self.ref_pos..self.ref_changes.len() {
            let change = &self.ref_changes[i];

            if change.idx < min_idx {
                self.ref_pos = i + 1;
                continue;
            }

            if change.color == target_color {
                self.b1_idx = i;
                break;
            }
        }
    }

    #[inline(always)]
    fn push_pixels(&mut self, count: usize) {
        // Clamp how many pixels we push so that we don't exceed the column
        // count for malformed files.
        let count = count.min(self.max_idx.saturating_sub(self.coding_line_len));
        let white = self.is_white;
        let byte_val: u8 = if white { 0xFF } else { 0x00 } ^ self.invert_mask;
        let mut remaining = count;

        // Fill partial byte buffer to boundary.
        while self.packer.has_pending() && remaining > 0 {
            if let Some(byte) = self.packer.push_bit(white) {
                self.decoder.push_byte(byte ^ self.invert_mask);
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
                self.decoder.push_byte(byte ^ self.invert_mask);
            }
        }

        // Track the color change:
        // - At start of line (no previous changes): only add if color differs from
        //   imaginary white (0), i.e., only add if black.
        // - Mid-line: only add if color differs from previous.
        if count > 0 {
            let color = self.cur_color();
            let is_change = self
                .coding_changes
                .last()
                .map_or(color != 0, |last| last.color != color);
            if is_change {
                self.coding_changes.push(ColorChange {
                    idx: self.coding_line_len,
                    color,
                });
            }
            self.coding_line_len += count;
        }
    }

    fn cur_color(&self) -> u8 {
        if self.is_white { 0 } else { 1 }
    }

    fn at_eol(&self) -> bool {
        self.a0().unwrap_or(0) == self.max_idx
    }

    #[inline(always)]
    fn next_line(&mut self, reader: &mut BitReader<'_>) -> Result<()> {
        // Go to next line.

        if self.coding_line_len != self.settings.columns as usize {
            return Err(DecodeError::LineLengthMismatch);
        }

        // Flush any partial byte with zero padding before finishing the line.
        if let Some(byte) = self.packer.flush() {
            self.decoder.push_byte(byte ^ self.invert_mask);
        }

        // Swap coding_changes into ref_changes for the next line.
        core::mem::swap(&mut self.ref_changes, &mut self.coding_changes);
        self.coding_changes.clear();
        self.coding_line_len = 0;
        self.ref_pos = 0;
        self.b1_idx = 0;
        self.is_white = true;
        self.decoded_rows += 1;
        self.decoder.next_line();

        if self.settings.rows_are_byte_aligned {
            reader.align();
        }

        self.update_b();

        Ok(())
    }
}
