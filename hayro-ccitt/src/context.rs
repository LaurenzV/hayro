use crate::bit_reader::BitReader;
use crate::codes::Mode;
use crate::{DecodeError, DecodeSettings, Decoder, Result};
use alloc::vec;
use alloc::vec::Vec;

/// A reusable context for decoding CCITT images.
pub struct DecoderContext {
    /// Positions of the color changes in the reference line (previous line).
    ///
    /// Since colors alternate and each line starts with white, changes at even
    /// indices are changes to black and changes at odd indices to white.
    ref_changes: Vec<u32>,
    /// The minimum index we need to start from when searching for b1.
    ref_pos: u32,
    /// The current index of b1.
    b1_idx: u32,
    /// Positions of the color changes in the coding line (current line being
    /// decoded).
    coding_changes: Vec<u32>,
    /// Current position in the coding line (number of pixels decoded).
    pub(crate) pixels_decoded: u32,
    /// The width of a line in pixels (i.e. number of columns).
    pub(crate) line_width: u32,
    /// The color of the next run to be decoded.
    pub(crate) color: Color,
    /// How many rows have been decoded so far.
    pub(crate) decoded_rows: u32,
    /// The settings to apply during decoding.
    pub(crate) settings: DecodeSettings,
    /// Whether the data contains EOLs.
    pub(crate) has_eols: bool,
}

impl DecoderContext {
    /// Creates a new decoder context using the provided decode settings.
    pub fn new(settings: DecodeSettings) -> Self {
        Self {
            ref_changes: vec![],
            ref_pos: 0,
            b1_idx: 0,
            coding_changes: Vec::new(),
            pixels_decoded: 0,
            line_width: settings.columns,
            // Each run starts with an imaginary white pixel on the left.
            color: Color::White,
            decoded_rows: 0,
            settings,
            has_eols: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.ref_changes.clear();
        self.ref_pos = 0;
        self.b1_idx = 0;
        self.coding_changes.clear();
        self.pixels_decoded = 0;
        self.line_width = self.settings.columns;
        self.color = Color::White;
        self.decoded_rows = 0;
        self.has_eols = false;
    }

    #[inline(always)]
    pub(crate) fn push_pixels(&mut self, count: u32) -> Result<()> {
        let remaining = self.line_width - self.pixels_decoded;

        if count > remaining && self.settings.strict {
            return Err(DecodeError::InvalidCode);
        }

        let count = count.min(remaining);

        if count > 0 {
            // Since colors alternate, an odd number of changes means that the
            // last pixel is black.
            let last_is_white = self.coding_changes.len().is_multiple_of(2);

            if last_is_white != self.color.is_white() {
                self.coding_changes.push(self.pixels_decoded);
            }

            self.pixels_decoded += count;
        }

        Ok(())
    }

    #[inline(always)]
    pub(crate) fn next_line(&mut self, decoder: &mut impl Decoder) {
        decoder.push_row(&self.coding_changes);
        core::mem::swap(&mut self.ref_changes, &mut self.coding_changes);
        self.coding_changes.clear();
        self.pixels_decoded = 0;
        self.ref_pos = 0;
        self.b1_idx = 0;
        self.color = Color::White;
        self.decoded_rows += 1;

        self.update_b();
    }

    /// `a0` refers to the first changing element on the current line.
    #[inline(always)]
    fn a0(&self) -> Option<u32> {
        if self.pixels_decoded == 0 {
            // If we haven't coded anything yet, a0 conceptually points at the
            // index -1. This is a bit of an edge case, and we therefore require
            // callers of this method to handle the case themselves.
            None
        } else {
            // Otherwise, the index points to the next element to be decoded.
            Some(self.pixels_decoded)
        }
    }

    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    #[inline(always)]
    fn b1(&self) -> u32 {
        self.ref_changes
            .get(self.b1_idx as usize)
            .copied()
            .unwrap_or(self.line_width)
    }

    /// "The next changing element to the right of b1, on the reference line."
    #[inline(always)]
    fn b2(&self) -> u32 {
        self.ref_changes
            .get(self.b1_idx as usize + 1)
            .copied()
            .unwrap_or(self.line_width)
    }

    /// Compute the new position of b1 (and implicitly b2).
    #[inline(always)]
    fn update_b(&mut self) {
        // b1 must be strictly greater than a0.
        let min_idx = self.a0().map_or(0, |a| a + 1);
        let mut idx = self.ref_pos as usize;

        while self.ref_changes.get(idx).is_some_and(|&c| c < min_idx) {
            idx += 1;
        }

        self.ref_pos = idx as u32;

        // b1 refers to an element of the opposite color, i.e. a change to black
        // (even index) if the current color is white and vice versa.
        if idx.is_multiple_of(2) != self.color.is_white() {
            idx += 1;
        }

        self.b1_idx = idx as u32;
    }

    #[inline(always)]
    fn at_eol(&self) -> bool {
        self.a0().unwrap_or(0) == self.line_width
    }
}

/// Decode a single 1D-coded line (T.4 Section 4.1.1, T.6 Section 2.2.4).
#[inline(always)]
pub(crate) fn decode_1d_line(ctx: &mut DecoderContext, reader: &mut BitReader<'_>) -> Result<()> {
    while !ctx.at_eol() {
        let run_length = reader.decode_run(ctx.color)?;
        ctx.push_pixels(run_length)?;
        ctx.color = ctx.color.opposite();
    }

    Ok(())
}

/// Decode a single 2D-coded line (T.4 Section 4.2, T.6 Section 2.2).
#[inline(always)]
pub(crate) fn decode_2d_line(ctx: &mut DecoderContext, reader: &mut BitReader<'_>) -> Result<()> {
    while !ctx.at_eol() {
        let mode = reader.decode_mode()?;

        match mode {
            // Pass mode (T.4 Section 4.2.1.3.2a, T.6 Section 2.2.3.1).
            Mode::Pass => {
                ctx.push_pixels(ctx.b2() - ctx.a0().unwrap_or(0))?;
                ctx.update_b();
                // No color change happens in pass mode.
            }
            // Vertical mode (T.4 Section 4.2.1.3.2b, T.6 Section 2.2.3.2).
            Mode::Vertical(i) => {
                let b1 = ctx.b1();
                let a1 = if i >= 0 {
                    b1.checked_add(i as u32).ok_or(DecodeError::Overflow)?
                } else {
                    b1.checked_sub((-i) as u32).ok_or(DecodeError::Overflow)?
                };

                let a0 = ctx.a0().unwrap_or(0);

                ctx.push_pixels(a1.checked_sub(a0).ok_or(DecodeError::Overflow)?)?;
                ctx.color = ctx.color.opposite();

                ctx.update_b();
            }
            // Horizontal mode (T.4 Section 4.2.1.3.2c, T.6 Section 2.2.3.3).
            Mode::Horizontal => {
                let a0a1 = reader.decode_run(ctx.color)?;
                ctx.push_pixels(a0a1)?;
                ctx.color = ctx.color.opposite();

                let a1a2 = reader.decode_run(ctx.color)?;
                ctx.push_pixels(a1a2)?;
                ctx.color = ctx.color.opposite();

                ctx.update_b();
            }
        }
    }

    Ok(())
}

/// Pixel color in a bi-level (black and white) image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Color {
    /// White pixel.
    White,
    /// Black pixel.
    Black,
}

impl Color {
    /// Returns the opposite color.
    #[inline(always)]
    fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    /// Returns true if this color is white.
    #[inline(always)]
    fn is_white(self) -> bool {
        matches!(self, Self::White)
    }
}
