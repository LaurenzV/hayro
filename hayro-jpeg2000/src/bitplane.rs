//! Decoding bitplanes into sample coefficients.
//!
//! Some of the references are taken from the "JPEG2000 Standard for Image Compression" book
//! instead of the specification.

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::packet::{CodeBlock, SubbandType};

pub(crate) struct BitplaneDecodeContext {
    /// The signs of each coefficient.
    signs: Vec<u8>,
    /// The magnitude of each coefficient that is successively built as we advance through the
    /// bitplanes.
    magnitude_array: Vec<ComponentBitPlanes>,
    /// The significance state of each coefficient. Will be set to one as soon as the
    /// first non-zero bit for that coefficient is encountered.
    significance_states: Vec<u8>,
    /// Whether the coefficient has previously had (at least one) magnitude refinement pass.
    first_magnitude_refinement: Vec<u8>,
    /// Whether the given coefficient belongs to a zero coding pass applied as part of sign
    /// propagation in the current bitplane. These values will be reset every time we advance to a
    /// new bitplane.
    has_zero_coding: Vec<u8>,
    /// The width of the code-block we are processing.
    width: u32,
    /// The height of the code-block we are processing.
    height: u32,
    /// The current type of subband that is being processed.
    subband_type: SubbandType,
    /// The arithmetic decoder contexts for each context label.
    contexts: [ArithmeticDecoderContext; 19],
}

impl BitplaneDecodeContext {
    pub(crate) fn new() -> Self {
        Self {
            signs: vec![],
            magnitude_array: vec![],
            significance_states: vec![],
            first_magnitude_refinement: vec![],
            has_zero_coding: vec![],
            width: 0,
            height: 0,
            subband_type: SubbandType::LowLow,
            contexts: [ArithmeticDecoderContext::default(); 19],
        }
    }

    fn set_sign(&mut self, pos: &Position, sign: u8) {
        self.signs[pos.index(self.width)] = sign;
    }

    fn ad_context(&mut self, ctx_label: u8) -> &mut ArithmeticDecoderContext {
        &mut self.contexts[ctx_label as usize]
    }

    fn reset_contexts(&mut self) {
        for context in &mut self.contexts {
            context.mps = 0;
            context.index = 0;
        }

        self.contexts[0].index = 4;
        self.contexts[17].index = 3;
        self.contexts[18].index = 46;
    }

    fn reset(&mut self, width: u32, height: u32, subband_type: SubbandType) {
        for arr in [
            &mut self.signs,
            &mut self.significance_states,
            &mut self.first_magnitude_refinement,
            &mut self.has_zero_coding,
        ] {
            arr.clear();
            arr.resize(width as usize * height as usize, 0);
        }

        self.magnitude_array.clear();
        self.magnitude_array.resize(
            width as usize * height as usize,
            ComponentBitPlanes::default(),
        );

        self.width = width;
        self.height = height;
        self.subband_type = subband_type;
        self.reset_contexts();
    }

    fn significance_state(&self, position: &Position) -> u8 {
        self.significance_states[position.index(self.width)]
    }

    fn is_magnitude_refined(&self, position: &Position) -> bool {
        self.first_magnitude_refinement[position.index(self.width)] != 0
    }

    fn has_zero_coding(&self, position: &Position) -> bool {
        self.has_zero_coding[position.index(self.width)] != 0
    }

    fn sign_checked(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            0
        } else {
            self.signs[x as usize + y as usize * self.width as usize]
        }
    }

    fn significance_state_checked(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            0
        } else {
            self.significance_state(&Position::new(x as u32, y as u32))
        }
    }

    /// The horizontal reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn horizontal_reference(&self, pos: &Position) -> u8 {
        self.significance_state_checked(pos.x as i64 - 1, pos.y as i64)
            + self.significance_state_checked(pos.x as i64 + 1, pos.y as i64)
    }

    /// The vertical reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn vertical_reference(&self, pos: &Position) -> u8 {
        self.significance_state_checked(pos.x as i64, pos.y as i64 - 1)
            + self.significance_state_checked(pos.x as i64, pos.y as i64 + 1)
    }

    /// The diagonal reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn diagonal_reference(&self, pos: &Position) -> u8 {
        self.significance_state_checked(pos.x as i64 - 1, pos.y as i64 - 1)
            + self.significance_state_checked(pos.x as i64 + 1, pos.y as i64 - 1)
            + self.significance_state_checked(pos.x as i64 - 1, pos.y as i64 + 1)
            + self.significance_state_checked(pos.x as i64 + 1, pos.y as i64 + 1)
    }
}

pub(crate) fn decode(code_block: &mut CodeBlock) -> Option<()> {
    Some(())
}

/// Perform the clean-up pass, specified in D.3.4.
/// See also the flow chart in Figure 7.3 in the JPEG2000 book.
fn cleanup_pass(ctx: &mut BitplaneDecodeContext, decoder: &mut ArithmeticDecoder) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);
    let mut cur_pos = position_iterator.next()?;

    loop {
        if ctx.significance_state(&cur_pos) == 0 && !ctx.has_zero_coding(&cur_pos) {
            let use_rl = cur_pos.y % 4 == 0 && (ctx.height - cur_pos.y) >= 4;

            let bit = if use_rl {
                unimplemented!();
            } else {
                let ctx_label = context_label_zero_coding(&cur_pos, &ctx);
                decoder.read_bit(ctx.ad_context(ctx_label))
            };
        }

        if let Some(next) = position_iterator.next() {
            cur_pos = next;
        } else {
            break;
        };
    }

    Some(())
}

fn decode_sign_bit(
    pos: &Position,
    ctx: &mut BitplaneDecodeContext,
    decoder: &mut ArithmeticDecoder,
) {
    let (ctx_label, xor_bit) = context_label_sign_coding(&pos, ctx);
    let ad_ctx = ctx.ad_context(ctx_label);
    let sign_bit = decoder.read_bit(ad_ctx) ^ xor_bit as u32;
    ctx.set_sign(pos, sign_bit as u8);
}

/// Table D.3.1.
///
/// Returns the context label.
fn context_label_zero_coding(pos: &Position, ctx: &BitplaneDecodeContext) -> u8 {
    let horizontal = ctx.horizontal_reference(pos);
    let vertical = ctx.vertical_reference(pos);
    let diagonal = ctx.diagonal_reference(pos);

    match ctx.subband_type {
        SubbandType::LowLow | SubbandType::LowHigh => {
            if horizontal == 2 {
                8
            } else if horizontal == 1 && vertical >= 1 {
                7
            } else if horizontal == 1 && vertical == 0 && diagonal >= 1 {
                6
            } else if horizontal == 1 && vertical == 0 && diagonal == 0 {
                5
            } else if horizontal == 0 && vertical == 2 {
                4
            } else if horizontal == 0 && vertical == 1 {
                3
            } else if horizontal == 0 && vertical == 0 && diagonal >= 2 {
                2
            } else if horizontal == 0 && vertical == 0 && diagonal == 1 {
                1
            } else {
                0
            }
        }
        SubbandType::HighLow => {
            if vertical == 2 {
                8
            } else if horizontal >= 1 && vertical == 1 {
                7
            } else if horizontal == 0 && vertical == 1 && diagonal >= 1 {
                6
            } else if horizontal == 0 && vertical == 1 && diagonal == 0 {
                5
            } else if horizontal == 2 && vertical == 0 {
                4
            } else if horizontal == 1 && vertical == 0 {
                3
            } else if horizontal == 0 && vertical == 0 && diagonal >= 2 {
                2
            } else if horizontal == 0 && vertical == 0 && diagonal == 1 {
                1
            } else {
                0
            }
        }
        SubbandType::HighHigh => {
            let hv = horizontal + vertical;

            if diagonal >= 3 {
                8
            } else if hv >= 1 && diagonal == 2 {
                7
            } else if hv == 0 && diagonal == 2 {
                6
            } else if hv >= 2 && diagonal == 1 {
                5
            } else if hv == 1 && diagonal == 1 {
                4
            } else if hv == 0 && diagonal == 1 {
                3
            } else if hv >= 2 && diagonal == 0 {
                2
            } else if hv == 1 && diagonal == 0 {
                1
            } else {
                0
            }
        }
    }
}

/// Table D.3.2.
///
/// Returns the context label as well as the X bit that needs to be XORed
/// with the next read bit.
fn context_label_sign_coding(pos: &Position, ctx: &BitplaneDecodeContext) -> (u8, u8) {
    fn neighbor_contribution(ctx: &BitplaneDecodeContext, x: i64, y: i64) -> i32 {
        let sigma = ctx.significance_state_checked(x, y);

        let multiplied = if ctx.sign_checked(x, y) == 0 { 1 } else { -1 };

        multiplied * sigma as i32
    }

    let h = (neighbor_contribution(ctx, pos.x as i64 - 1, pos.y as i64)
        + neighbor_contribution(ctx, pos.x as i64 + 1, pos.y as i64))
    .clamp(-1, 1);
    let v = (neighbor_contribution(ctx, pos.x as i64, pos.y as i64 - 1)
        + neighbor_contribution(ctx, pos.x as i64, pos.y as i64 + 1))
    .clamp(-1, 1);

    match (h, v) {
        (1, 1) => (13, 0),
        (1, 0) => (12, 0),
        (1, -1) => (11, 0),
        (0, 1) => (10, 0),
        (0, 0) => (9, 0),
        (0, -1) => (10, 1),
        (-1, 1) => (11, 1),
        (-1, 0) => (12, 1),
        (-1, -1) => (13, 1),
        _ => unreachable!(),
    }
}

/// Table D.4.
///
/// Returns the context label.
fn context_label_magnitude_refinement_coding(pos: &Position, ctx: &BitplaneDecodeContext) -> u8 {
    if ctx.is_magnitude_refined(pos) {
        16
    } else {
        let summed = ctx.horizontal_reference(pos)
            + ctx.vertical_reference(pos)
            + ctx.diagonal_reference(pos);

        if summed >= 1 { 15 } else { 14 }
    }
}

#[derive(Default, Copy, Clone)]
struct ComponentBitPlanes {
    inner: u8,
    count: u8,
}

impl ComponentBitPlanes {
    fn push_bit(&mut self, bit: u8) {
        assert!(self.count < 8);

        self.inner = (self.inner << 1) | bit & 1;
    }
}

#[derive(Default, Copy, Clone, Debug)]
struct Position {
    x: u32,
    y: u32,
}

impl Position {
    fn new(x: u32, y: u32) -> Position {
        Self { x, y }
    }

    fn index(&self, width: u32) -> usize {
        self.x as usize + self.y as usize * width as usize
    }
}

struct PositionIterator {
    cur_row: u32,
    position: Position,
    width: u32,
    height: u32,
}

impl PositionIterator {
    fn new(width: u32, height: u32) -> Self {
        Self {
            cur_row: 0,
            position: Position::default(),
            width,
            height,
        }
    }

    fn reset(&mut self) {
        self.cur_row = 0;
        self.position = Position::default();
    }

    fn has_4_columns(&self) -> bool {
        self.height - self.cur_row >= 4
    }
}

impl Iterator for PositionIterator {
    type Item = Position;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position.y >= self.height || self.position.y == self.cur_row + 4 {
            self.position.x += 1;
            self.position.y = self.cur_row;
        }

        if self.position.x >= self.width {
            self.position.x = 0;
            self.cur_row += 4;
            self.position.y = self.cur_row;
        }

        if self.position.y >= self.height {
            return None;
        }

        let pos = self.position;

        self.position.y += 1;
        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::PositionIterator;

    macro_rules! pt {
        ($x:expr, $y:expr) => {
            ($x as u32, $y as u32)
        };
    }

    #[test]
    fn position_iterator() {
        let width = 5;
        let height = 10;

        let mut iter = PositionIterator::new(width, height);
        let mut produced = Vec::new();

        while let Some(position) = iter.next() {
            produced.push((position.x, position.y));
        }

        #[rustfmt::skip]
        let expected = [
            pt!(0, 0), pt!(0, 1), pt!(0, 2), pt!(0, 3),
            pt!(1, 0), pt!(1, 1), pt!(1, 2), pt!(1, 3),
            pt!(2, 0), pt!(2, 1), pt!(2, 2), pt!(2, 3),
            pt!(3, 0), pt!(3, 1), pt!(3, 2), pt!(3, 3),
            pt!(4, 0), pt!(4, 1), pt!(4, 2), pt!(4, 3),
            pt!(0, 4), pt!(0, 5), pt!(0, 6), pt!(0, 7),
            pt!(1, 4), pt!(1, 5), pt!(1, 6), pt!(1, 7),
            pt!(2, 4), pt!(2, 5), pt!(2, 6), pt!(2, 7),
            pt!(3, 4), pt!(3, 5), pt!(3, 6), pt!(3, 7),
            pt!(4, 4), pt!(4, 5), pt!(4, 6), pt!(4, 7),
            pt!(0, 8), pt!(0, 9), pt!(1, 8), pt!(1, 9),
            pt!(2, 8), pt!(2, 9), pt!(3, 8), pt!(3, 9),
            pt!(4, 8), pt!(4, 9)
        ];

        assert_eq!(produced.as_slice(), &expected);
    }
}
