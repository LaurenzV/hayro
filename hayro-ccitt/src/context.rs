use crate::bit_reader::BitReader;
use crate::codes::Mode;
use crate::{DecodeError, DecodeSettings, Decoder, Result};
use alloc::vec;
use alloc::vec::Vec;

pub(crate) struct DecoderContext<'a> {
    // Even indices are changes to black, odd ones to white.
    ref_changes: Vec<u32>,
    ref_pos: u32,
    b1_idx: u32,
    coding_changes: Vec<u32>,
    pub(crate) pixels_decoded: u32,
    pub(crate) line_width: u32,
    pub(crate) color: Color,
    pub(crate) decoded_rows: u32,
    pub(crate) settings: &'a DecodeSettings,
    pub(crate) has_eols: bool,
}

impl<'a> DecoderContext<'a> {
    pub(crate) fn new(settings: &'a DecodeSettings) -> Self {
        Self {
            ref_changes: vec![],
            ref_pos: 0,
            b1_idx: 0,
            coding_changes: Vec::new(),
            pixels_decoded: 0,
            line_width: settings.columns,
            color: Color::White,
            decoded_rows: 0,
            settings,
            has_eols: false,
        }
    }

    #[inline(always)]
    pub(crate) fn push_pixels(&mut self, count: u32) -> Result<()> {
        let remaining = self.line_width - self.pixels_decoded;

        if count > remaining && self.settings.strict {
            return Err(DecodeError::InvalidCode);
        }

        let count = count.min(remaining);

        if count > 0 {
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

    #[inline(always)]
    fn a0(&self) -> Option<u32> {
        if self.pixels_decoded == 0 {
            None
        } else {
            Some(self.pixels_decoded)
        }
    }

    #[inline(always)]
    fn b1(&self) -> u32 {
        self.ref_changes
            .get(self.b1_idx as usize)
            .copied()
            .unwrap_or(self.line_width)
    }

    #[inline(always)]
    fn b2(&self) -> u32 {
        self.ref_changes
            .get(self.b1_idx as usize + 1)
            .copied()
            .unwrap_or(self.line_width)
    }

    #[inline(always)]
    fn update_b(&mut self) {
        let min_idx = self.a0().map_or(0, |a| a + 1);
        let mut idx = self.ref_pos as usize;

        while self.ref_changes.get(idx).is_some_and(|&c| c < min_idx) {
            idx += 1;
        }

        self.ref_pos = idx as u32;

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

#[inline(always)]
pub(crate) fn decode_1d_line(
    ctx: &mut DecoderContext<'_>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    while !ctx.at_eol() {
        let run_length = reader.decode_run(ctx.color)?;
        ctx.push_pixels(run_length)?;
        ctx.color = ctx.color.opposite();
    }

    Ok(())
}

#[inline(always)]
pub(crate) fn decode_2d_line(
    ctx: &mut DecoderContext<'_>,
    reader: &mut BitReader<'_>,
) -> Result<()> {
    while !ctx.at_eol() {
        let mode = reader.decode_mode()?;

        match mode {
            Mode::Pass => {
                ctx.push_pixels(ctx.b2() - ctx.a0().unwrap_or(0))?;
                ctx.update_b();
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Color {
    White,
    Black,
}

impl Color {
    #[inline(always)]
    fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    #[inline(always)]
    fn is_white(self) -> bool {
        matches!(self, Self::White)
    }
}
