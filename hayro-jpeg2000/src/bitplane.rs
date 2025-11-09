//! Bitplane decoding, described in Annex D.
//!
//! JPEG2000 groups the samples of each component into their constituent
//! bit planes and uses a special context-modeling approach to encode the
//! bits using the arithmetic encoder. In this stage, we need to "revert" the
//! context-modeling so that we can extract the magnitudes and signs of each
//! sample.
//!
//! Some of the references are taken from the
//! "JPEG2000 Standard for Image Compression" book instead of the specification.

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::codestream::CodeBlockStyle;
use crate::decode::{CodeBlock, Layer, Segment, SubBandType};
use log::warn;

/// Decode the layers of the given code block into coefficients.
///
/// The result will be stored in the form of a vector of signs and magnitudes
/// in the bitplane decoder context.
pub(crate) fn decode(
    code_block: &CodeBlock,
    sub_band_type: SubBandType,
    num_bitplanes: u16,
    style: &CodeBlockStyle,
    ctx: &mut CodeBlockDecodeContext,
    layers: &[Layer],
    all_segments: &[Segment],
) -> Result<(), &'static str> {
    ctx.reset(code_block, sub_band_type, style);

    if code_block.number_of_coding_passes == 0 {
        return Ok(());
    }

    if num_bitplanes > 32 {
        // If we want to adjust this, we need to change how `ComponentBits`
        // works.
        return Err("bitplanes with more than 32 bits are not supported");
    }

    let mut layer_buffer = std::mem::take(&mut ctx.layer_buffer).unwrap_or_default();
    layer_buffer.clear();

    if style.selective_arithmetic_coding_bypass {
        return Err("unsupported code-block style features encountered during decoding");
    }

    decode_inner(code_block, style, num_bitplanes, layers, all_segments, ctx)
        .ok_or("failed to decode code-block arithmetic data")?;

    ctx.layer_buffer = Some(layer_buffer);

    Ok(())
}

fn decode_inner(
    code_block: &CodeBlock,
    style: &CodeBlockStyle,
    num_bitplanes: u16,
    layers: &[Layer],
    all_segments: &[Segment],
    ctx: &mut CodeBlockDecodeContext,
) -> Option<()> {
    let mut combined_layers = vec![];
    let mut segment_ranges = vec![0];

    for layer in layers {
        if let Some(range) = layer.segments.clone() {
            for segment in &all_segments[range.clone()] {
                combined_layers.extend(segment.data);
                segment_ranges.push(combined_layers.len());
            }
        }
    }

    let mut decoder = if style.termination_on_each_pass {
        ArithmeticDecoder::new(&combined_layers[..segment_ranges[1]])
    } else {
        ArithmeticDecoder::new(&combined_layers)
    };

    for coding_pass in 0..code_block.number_of_coding_passes {
        if coding_pass > 0 && style.termination_on_each_pass {
            let data = &combined_layers
                [segment_ranges[coding_pass as usize]..segment_ranges[coding_pass as usize + 1]];
            decoder = ArithmeticDecoder::new(data);
        }

        enum PassType {
            Cleanup,
            SignificancePropagation,
            MagnitudeRefinement,
        }

        // The first bitplane only has a cleanup pass, all other bitplanes
        // are in the order SPP -> MRR -> C.
        let pass = match coding_pass % 3 {
            0 => PassType::Cleanup,
            1 => PassType::SignificancePropagation,
            2 => PassType::MagnitudeRefinement,
            _ => unreachable!(),
        };

        match pass {
            PassType::Cleanup => {
                cleanup_pass(ctx, &mut decoder);

                if style.segmentation_symbols {
                    let b0 = decoder.read_bit(ctx.arithmetic_decoder_context(18));
                    let b1 = decoder.read_bit(ctx.arithmetic_decoder_context(18));
                    let b2 = decoder.read_bit(ctx.arithmetic_decoder_context(18));
                    let b3 = decoder.read_bit(ctx.arithmetic_decoder_context(18));

                    if b0 != 1 || b1 != 0 || b2 != 1 || b3 != 0 {
                        warn!("encountered invalid segmentation symbol");
                        return None;
                    }
                }

                ctx.reset_for_next_bitplane();
            }
            PassType::SignificancePropagation => {
                significance_propagation_pass(ctx, &mut decoder);
            }
            PassType::MagnitudeRefinement => {
                magnitude_refinement_pass(ctx, &mut decoder);
            }
        }

        if style.reset_context_probabilities {
            ctx.reset_contexts();
        }
    }

    // Extend all coefficients with zero bits until we have the required number
    // of bits.
    for el in &mut ctx.magnitude_array {
        while (el.count as u16) < num_bitplanes {
            el.push_bit(0);
        }
    }

    Some(())
}

pub(crate) struct CodeBlockDecodeContext {
    /// The signs of each coefficient.
    signs: Vec<u8>,
    /// The magnitude of each coefficient that is successively built as we advance through the
    /// bitplanes.
    magnitude_array: Vec<ComponentBits>,
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
    /// Whether the vertical causal flag is enabled.
    vertically_causal: bool,
    /// The type of sub-band the current code block belongs to.
    sub_band_type: SubBandType,
    /// The arithmetic decoder contexts for each context label.
    contexts: [ArithmeticDecoderContext; 19],
    /// A buffer used for concatenating the data of layers for a single codeblock.
    /// The allocation will be taken out at the beginning of a decode operation
    /// (leaving `None` in place) and be put back after decoding.
    layer_buffer: Option<Vec<u8>>,
}

impl Default for CodeBlockDecodeContext {
    fn default() -> Self {
        Self {
            signs: vec![],
            magnitude_array: vec![],
            significance_states: vec![],
            first_magnitude_refinement: vec![],
            has_zero_coding: vec![],
            width: 0,
            height: 0,
            vertically_causal: false,
            sub_band_type: SubBandType::LowLow,
            contexts: [ArithmeticDecoderContext::default(); 19],
            layer_buffer: Some(vec![]),
        }
    }
}

impl CodeBlockDecodeContext {
    /// Completely reset context so that it can be reused for a new code-block.
    pub(crate) fn reset(
        &mut self,
        code_block: &CodeBlock,
        sub_band_type: SubBandType,
        code_block_style: &CodeBlockStyle,
    ) {
        let (width, height) = (code_block.rect.width(), code_block.rect.height());

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
        self.magnitude_array
            .resize(width as usize * height as usize, ComponentBits::default());

        for mag in &mut self.magnitude_array {
            mag.count = code_block.missing_bit_planes;
        }

        self.width = width;
        self.height = height;
        self.sub_band_type = sub_band_type;
        self.vertically_causal = code_block_style.vertically_causal_context;
        self.reset_contexts();
    }

    pub(crate) fn signs(&self) -> &[u8] {
        &self.signs
    }

    pub(crate) fn magnitudes(&self) -> &[ComponentBits] {
        &self.magnitude_array
    }

    fn set_sign(&mut self, pos: &Position, sign: u8) {
        self.signs[pos.index(self.width)] = sign;
    }

    fn arithmetic_decoder_context(&mut self, ctx_label: u8) -> &mut ArithmeticDecoderContext {
        &mut self.contexts[ctx_label as usize]
    }

    /// Reset each context to the initial state defined in table D.7.
    fn reset_contexts(&mut self) {
        for context in &mut self.contexts {
            context.mps = 0;
            context.index = 0;
        }

        self.contexts[0].index = 4;
        self.contexts[17].index = 3;
        self.contexts[18].index = 46;
    }

    fn reset_for_next_bitplane(&mut self) {
        self.has_zero_coding = vec![0; self.has_zero_coding.len()];
    }

    fn significance_state(&self, position: &Position) -> u8 {
        self.significance_states[position.index(self.width)]
    }

    fn is_significant(&self, position: &Position) -> bool {
        self.significance_states[position.index(self.width)] != 0
    }

    fn set_significant(&mut self, position: &Position) {
        self.significance_states[position.index(self.width)] = 1;
    }

    fn set_zero_coded(&mut self, position: &Position) {
        self.has_zero_coding[position.index(self.width)] = 1;
    }

    fn set_magnitude_refined(&mut self, position: &Position) {
        self.first_magnitude_refinement[position.index(self.width)] = 1;
    }

    fn is_magnitude_refined(&self, position: &Position) -> bool {
        self.first_magnitude_refinement[position.index(self.width)] != 0
    }

    fn is_zero_coded(&self, position: &Position) -> bool {
        self.has_zero_coding[position.index(self.width)] != 0
    }

    fn push_magnitude_bit(&mut self, position: &Position, bit: u32) {
        self.magnitude_array[position.index(self.width)].push_bit(bit)
    }

    #[inline]
    fn sign_checked(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            // OOB values should just return 0.
            0
        } else {
            self.signs[x as usize + y as usize * self.width as usize]
        }
    }

    #[inline]
    fn significance_state_checked(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            // OOB values should just return 0.
            0
        } else {
            self.significance_state(&Position::new(x as u32, y as u32))
        }
    }

    #[inline]
    fn neighbor_in_next_stripe(&self, pos: &Position, neighbor_y: u32) -> bool {
        neighbor_y < self.height && (neighbor_y >> 2) > (pos.y >> 2)
    }

    #[inline]
    fn horizontal_significance_states(&self, pos: &Position) -> u8 {
        self.significance_state_checked(pos.x as i64 - 1, pos.y as i64)
            + self.significance_state_checked(pos.x as i64 + 1, pos.y as i64)
    }

    #[inline]
    fn vertical_significance_states(&self, pos: &Position) -> u8 {
        let suppress_lower = self.vertically_causal && self.neighbor_in_next_stripe(pos, pos.y + 1);

        self.significance_state_checked(pos.x as i64, pos.y as i64 - 1)
            + if suppress_lower {
                0
            } else {
                self.significance_state_checked(pos.x as i64, pos.y as i64 + 1)
            }
    }

    #[inline(always)]
    fn diagonal_significance_states(&self, pos: &Position) -> u8 {
        let suppress_lower = self.vertically_causal && self.neighbor_in_next_stripe(pos, pos.y + 1);

        self.significance_state_checked(pos.x as i64 - 1, pos.y as i64 - 1)
            + self.significance_state_checked(pos.x as i64 + 1, pos.y as i64 - 1)
            + if suppress_lower {
                0
            } else {
                self.significance_state_checked(pos.x as i64 - 1, pos.y as i64 + 1)
            }
            + if suppress_lower {
                0
            } else {
                self.significance_state_checked(pos.x as i64 + 1, pos.y as i64 + 1)
            }
    }

    #[inline]
    fn neighborhood_significance_states(&self, pos: &Position) -> u8 {
        self.horizontal_significance_states(pos)
            + self.vertical_significance_states(pos)
            + self.diagonal_significance_states(pos)
    }
}

/// Perform the cleanup pass, specified in D.3.4.
/// See also the flow chart in Figure 7.3 in the JPEG2000 book.
fn cleanup_pass(ctx: &mut CodeBlockDecodeContext, decoder: &mut impl BitDecoder) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(mut cur_pos) = position_iterator.next() else {
            break;
        };

        if !ctx.is_significant(&cur_pos) && !ctx.is_zero_coded(&cur_pos) {
            let use_rl = cur_pos.y % 4 == 0
                && (ctx.height - cur_pos.y) >= 4
                && ctx.neighborhood_significance_states(&cur_pos) == 0
                && ctx.neighborhood_significance_states(&Position::new(cur_pos.x, cur_pos.y + 1))
                    == 0
                && ctx.neighborhood_significance_states(&Position::new(cur_pos.x, cur_pos.y + 2))
                    == 0
                && ctx.neighborhood_significance_states(&Position::new(cur_pos.x, cur_pos.y + 3))
                    == 0;

            let bit = if use_rl {
                // "If the four contiguous coefficients in the column being scanned are all decoded
                // in the cleanup pass and the context label for all is 0 (including context
                // coefficients from previous magnitude, significance and cleanup passes), then the
                // unique run-length context is given to the arithmetic decoder along with the bit
                // stream."
                let bit = decoder.read_bit(ctx.arithmetic_decoder_context(17));

                if bit == 0 {
                    // "If the symbol 0 is returned, then all four contiguous coefficients in
                    // the column remain insignificant and are set to zero."
                    ctx.push_magnitude_bit(&cur_pos, 0);

                    for _ in 0..3 {
                        cur_pos = position_iterator.next()?;
                        ctx.push_magnitude_bit(&cur_pos, 0);
                    }

                    continue;
                } else {
                    // "Otherwise, if the symbol 1 is returned, then at least
                    // one of the four contiguous coefficients in the column is
                    // significant. The next two bits, returned with the
                    // UNIFORM context (index 46 in Table C.2), denote which
                    // coefficient from the top of the column down is the first
                    // to be found significant."
                    let mut num_zeroes = decoder.read_bit(ctx.arithmetic_decoder_context(18));
                    num_zeroes =
                        (num_zeroes << 1) | decoder.read_bit(ctx.arithmetic_decoder_context(18));

                    for _ in 0..num_zeroes {
                        ctx.push_magnitude_bit(&cur_pos, 0);
                        cur_pos = position_iterator.next()?;
                    }

                    1
                }
            } else {
                let ctx_label = context_label_zero_coding(&cur_pos, ctx);
                decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))
            };

            ctx.push_magnitude_bit(&cur_pos, bit);

            if bit == 1 {
                decode_sign_bit(&cur_pos, ctx, decoder);
                ctx.set_significant(&cur_pos);
            }
        }
    }

    Some(())
}

/// Perform the significance propagation pass (Section D.3.1).
///
/// See also the flow chart in Figure 7.4 in the JPEG2000 book.
fn significance_propagation_pass(
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(cur_pos) = position_iterator.next() else {
            break;
        };

        // "The significance propagation pass only includes bits of coefficients
        // that were insignificant (the significance state has yet to be set)
        // and have a non-zero context."
        if !ctx.is_significant(&cur_pos) && ctx.neighborhood_significance_states(&cur_pos) != 0 {
            let ctx_label = context_label_zero_coding(&cur_pos, ctx);
            let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label));
            ctx.push_magnitude_bit(&cur_pos, bit);
            ctx.set_zero_coded(&cur_pos);

            // "If the value of this bit is 1 then the significance
            // state is set to 1 and the immediate next bit to be decoded is
            // the sign bit for the coefficient. Otherwise, the significance
            // state remains 0."
            if bit == 1 {
                decode_sign_bit(&cur_pos, ctx, decoder);
                ctx.set_significant(&cur_pos);
            }
        }
    }

    Some(())
}

/// Perform the magnitude refinement pass, specified in Section D.3.3.
///
/// See also the flow chart in Figure 7.5 in the JPEG2000 book.
fn magnitude_refinement_pass(
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(cur_pos) = position_iterator.next() else {
            break;
        };

        if ctx.is_significant(&cur_pos) && !ctx.is_zero_coded(&cur_pos) {
            let ctx_label = context_label_magnitude_refinement_coding(&cur_pos, ctx);
            let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label));
            ctx.push_magnitude_bit(&cur_pos, bit);
            ctx.set_magnitude_refined(&cur_pos);
        }
    }

    Some(())
}

/// Decode a sign bit (Section D.3.2).
#[inline(always)]
fn decode_sign_bit(
    pos: &Position,
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
) {
    /// Based on Table D.2.
    #[inline(always)]
    fn context_label_sign_coding(pos: &Position, ctx: &CodeBlockDecodeContext) -> (u8, u8) {
        #[inline(always)]
        fn neighbor_contribution(ctx: &CodeBlockDecodeContext, x: i64, y: i64) -> i32 {
            let sigma = ctx.significance_state_checked(x, y);

            let multiplied = if ctx.sign_checked(x, y) == 0 { 1 } else { -1 };

            multiplied * sigma as i32
        }

        let h = (neighbor_contribution(ctx, pos.x as i64 - 1, pos.y as i64)
            + neighbor_contribution(ctx, pos.x as i64 + 1, pos.y as i64))
        .clamp(-1, 1);
        let suppress_lower = ctx.vertically_causal && ctx.neighbor_in_next_stripe(pos, pos.y + 1);
        let v = (neighbor_contribution(ctx, pos.x as i64, pos.y as i64 - 1)
            + if suppress_lower {
                0
            } else {
                neighbor_contribution(ctx, pos.x as i64, pos.y as i64 + 1)
            })
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

    let (ctx_label, xor_bit) = context_label_sign_coding(pos, ctx);
    let ad_ctx = ctx.arithmetic_decoder_context(ctx_label);
    let sign_bit = decoder.read_bit(ad_ctx) ^ xor_bit as u32;
    ctx.set_sign(pos, sign_bit as u8);
}

/// Return the context label for zero coding (Section D.3.1).
#[inline(always)]
fn context_label_zero_coding(pos: &Position, ctx: &CodeBlockDecodeContext) -> u8 {
    let mut horizontal = ctx.horizontal_significance_states(pos);
    let mut vertical = ctx.vertical_significance_states(pos);
    let diagonal = ctx.diagonal_significance_states(pos);

    match ctx.sub_band_type {
        SubBandType::LowLow | SubBandType::LowHigh | SubBandType::HighLow => {
            if ctx.sub_band_type == SubBandType::HighLow {
                std::mem::swap(&mut horizontal, &mut vertical);
            }

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
        SubBandType::HighHigh => {
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

/// Return the context label for magnitude refinement coding (Table D.4).
fn context_label_magnitude_refinement_coding(pos: &Position, ctx: &CodeBlockDecodeContext) -> u8 {
    if ctx.is_magnitude_refined(pos) {
        16
    } else {
        let summed = ctx.horizontal_significance_states(pos)
            + ctx.vertical_significance_states(pos)
            + ctx.diagonal_significance_states(pos);

        if summed >= 1 { 15 } else { 14 }
    }
}

#[derive(Default, Copy, Clone, Debug)]
pub(crate) struct ComponentBits {
    inner: u32,
    count: u8,
}

impl ComponentBits {
    fn push_bit(&mut self, bit: u32) {
        assert!(self.count < 32);
        assert!(bit < 2);

        self.inner = (self.inner << 1) | bit;
        self.count += 1;
    }

    pub(crate) fn get(&self) -> u32 {
        self.inner
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
}

impl Iterator for PositionIterator {
    type Item = Position;

    fn next(&mut self) -> Option<Self::Item> {
        // "Each bit-plane of a code-block is scanned in a particular order.
        // Starting at the top left, the first four coefficients of the
        // first column are scanned, followed by the first four coefficients of
        // the second column and so on, until the right side of the code-block
        // is reached. The scan then returns to the left of the code-block and
        // the second set of four coefficients in each column is scanned. The
        // process is continued to the bottom of the code-block. If the
        // code-block height is not divisible by 4, the last set of coefficients
        // scanned in each column will contain fewer than 4 members."

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

// We use a trait so that we can mock the arithmetic decoder for tests.
trait BitDecoder {
    fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32;
}

impl BitDecoder for ArithmeticDecoder<'_> {
    fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        Self::read_bit(self, context)
    }
}

#[cfg(test)]
mod tests {
    use super::{CodeBlockDecodeContext, PositionIterator, decode};
    use crate::codestream::CodeBlockStyle;
    use crate::decode::{CodeBlock, Layer, Segment, SubBandType};
    use crate::rect::IntRect;

    impl CodeBlockDecodeContext {
        fn coefficients(&self) -> Vec<i32> {
            let mut coefficients = vec![];

            for (c, sign) in self.magnitudes().iter().zip(self.signs.iter()) {
                let mut res = c.get() as i32;

                if *sign != 0 {
                    res = -res;
                }

                coefficients.push(res);
            }

            coefficients
        }
    }

    macro_rules! pt {
        ($x:expr, $y:expr) => {
            ($x as u32, $y as u32)
        };
    }

    #[test]
    fn position_iterator() {
        let width = 5;
        let height = 10;

        let iter = PositionIterator::new(width, height);
        let mut produced = Vec::new();

        for position in iter {
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

    // First packet from example in Section J.10.4.
    #[test]
    fn bitplane_decoding_2() {
        let data = [0x01, 0x8f, 0x0d, 0xc8, 0x75, 0x5d];

        let code_block = CodeBlock {
            rect: IntRect::from_xywh(0, 0, 1, 5),
            x_idx: 0,
            y_idx: 0,
            layers: 0..1,
            has_been_included: false,
            missing_bit_planes: 0,
            number_of_coding_passes: 16,
            l_block: 0,
        };

        let mut ctx = CodeBlockDecodeContext::default();

        decode(
            &code_block,
            SubBandType::LowLow,
            6,
            &CodeBlockStyle::default(),
            &mut ctx,
            &[Layer {
                segments: Some(0..1),
            }],
            &[Segment {
                length: data.len() as u32,
                data: &data,
            }],
        )
        .unwrap();

        let coefficients = ctx.coefficients();

        assert_eq!(coefficients, vec![-26, -22, -30, -32, -19]);
    }

    // Second packet from example in Section J.10.4.
    #[test]
    fn bitplane_decoding_3() {
        let data = [0x0F, 0xB1, 0x76];

        let code_block = CodeBlock {
            rect: IntRect::from_xywh(0, 0, 1, 4),
            x_idx: 0,
            y_idx: 0,
            layers: 0..1,
            has_been_included: false,
            missing_bit_planes: 0,
            number_of_coding_passes: 7,
            l_block: 0,
        };

        let mut ctx = CodeBlockDecodeContext::default();

        decode(
            &code_block,
            SubBandType::LowHigh,
            3,
            &CodeBlockStyle::default(),
            &mut ctx,
            &[Layer {
                segments: Some(0..1),
            }],
            &[Segment {
                length: data.len() as u32,
                data: &data,
            }],
        )
        .unwrap();

        let coefficients = ctx.coefficients();

        assert_eq!(coefficients, vec![1, 5, 1, 0]);
    }

    // Second packet from example in Section J.10.4.
    #[test]
    fn bitplane_decoding_debug() {
        let data = vec![
            225, 72, 111, 59, 122, 13, 70, 63, 48, 1, 128, 138, 167, 142, 136, 234, 176, 18, 250,
            155, 201, 209, 178, 22, 3, 122, 65, 71, 189, 9, 116, 133, 67, 58, 236, 36, 96, 180,
            149, 176, 210, 225, 171, 223, 90, 253, 30, 222, 151, 102, 39, 30, 60, 157, 116, 17, 8,
            141, 68, 131, 67, 132, 26, 211, 205, 234, 114, 234, 111, 228, 220, 77, 234, 216, 84, 2,
            25, 142, 108, 246, 245, 33, 60, 206, 71, 9, 179, 66, 149, 216, 164, 135, 42, 146, 104,
            78, 63, 79, 112, 108, 108, 114, 239, 235, 88, 168, 87, 191, 194, 236, 134, 79, 1, 98,
            61, 204, 148, 226, 181, 124, 207, 254, 19, 70, 229, 25, 35, 118, 148, 10, 123, 207,
            148, 214, 75, 143, 254, 109, 78, 34, 254, 242, 12, 97, 100, 199, 130, 49, 4, 67, 50,
            32, 3, 98, 70, 155, 104, 103, 90, 193, 89, 59, 68, 148, 110, 7, 3, 141, 178, 237, 93,
            253, 5, 69, 137, 207, 188, 149, 131, 59, 203, 223, 41, 106, 78, 51, 223, 21, 113, 99,
            204, 208, 145, 44, 51, 14, 133, 90, 118, 136, 134, 167, 54, 22, 84, 84, 47, 206, 125,
            89, 39, 60, 52, 175, 97, 228, 217, 133, 171, 135, 129, 201, 164, 82, 3, 110, 200, 88,
            1, 140, 235, 79, 57, 38, 185, 197, 236, 33, 222, 117, 107, 156, 18, 78, 235, 63, 131,
            57, 197, 153, 196, 178, 254, 161, 28, 72, 103, 42, 31, 255, 56, 2, 18, 126, 95, 98, 19,
            30, 233,
        ];

        let code_block = CodeBlock {
            rect: IntRect::from_xywh(0, 0, 32, 32),
            x_idx: 0,
            y_idx: 0,
            layers: 0..1,
            has_been_included: false,
            missing_bit_planes: 5,
            number_of_coding_passes: 13,
            l_block: 0,
        };

        let mut ctx = CodeBlockDecodeContext::default();

        decode(
            &code_block,
            SubBandType::HighLow,
            5,
            &CodeBlockStyle::default(),
            &mut ctx,
            &[Layer {
                segments: Some(0..1),
            }],
            &[Segment {
                length: data.len() as u32,
                data: &data,
            }],
        )
        .unwrap();

        let coefficients = ctx.coefficients();

        let expected = vec![
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1, 0, -2, 0, -1, 0, 1, 1, -1, 0, 0,
            0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0,
            2, 0, 0, 0, 1, 3, -2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0,
            0, 0, -1, 0, -2, -1, -2, -1, -1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1, -1, 0, 0,
            -1, 0, -1, 1, 1, 0, 0, 0, 0, 0, 1, 1, -1, -2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 1, 0, 0, -1, 0, -1, 2, 1, 0, 1, 1, -1, 0, -2, 1, 4, -1, 0, 1, -1, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 1, 0, 0, 0, 1, -1, 1, 0, 0, 0, 0, 1, 1, 1, 2, -3, 2, 1, 1, -1, -1, 0, 0, 0,
            0, 0, 0, 0, 0, -1, -1, 0, 0, 0, 0, -1, 0, 1, -1, -1, 1, 1, 0, 1, 1, 0, -1, 3, -1, 1, 2,
            0, 2, 0, 0, 0, 0, 0, 0, 0, -1, -1, 1, 1, 0, 0, 0, 0, 0, 0, 0, -1, 1, 2, 0, -2, -1, -1,
            1, 1, 0, -2, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 2, 1, 0, 1, 1,
            0, 0, -1, 1, -1, 0, 2, 2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 2, 1, 0,
            1, 0, 1, 0, -1, 0, 1, -2, -1, -3, -2, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, -1, 0, 0, -1, -1,
            0, 0, 0, -1, 0, 0, 0, -2, 2, 1, -3, 0, 0, 0, 1, 0, -2, 0, 0, 0, -1, 0, 0, 0, 0, 1, -1,
            0, 1, 0, 1, 1, 0, 0, 0, 1, 0, 0, 1, 1, 1, -3, 2, -1, 2, 0, 1, 1, 1, 0, 0, 2, 0, 0, 0,
            0, 0, 1, 0, 0, 0, -1, 0, -1, 0, 1, 1, 0, -1, 0, 1, 1, -3, 1, -1, -1, 3, 3, 1, 1, 0, 1,
            1, 0, 2, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, -1, 0, 0, -2, 0, 1, 0, -2, 0, 1,
            1, 3, 2, 0, 1, 1, 1, -1, 0, 0, 0, 0, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1, 3, 0, 5, 1, 3, 0,
            -1, 2, 3, -1, -2, 0, 2, 2, 0, 1, 1, -1, -1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 1, 0, 2, 0, -5,
            2, -2, 0, -3, 0, -3, 1, 1, 0, -1, 0, 0, 2, 2, -2, -1, -1, 1, -1, 0, 1, -1, 0, 1, 0, 0,
            0, 0, 0, -1, 3, 2, 1, 2, 0, -1, 0, -2, 2, 0, -1, -1, -1, 0, 0, 0, 2, 0, 0, 1, 0, 1, 0,
            0, 1, -1, -1, 1, 0, -1, -3, 3, 1, -1, 0, -1, 0, 1, 2, 0, 1, 1, 0, 0, 1, 1, -2, -1, 0,
            -2, 1, 0, -1, -1, 0, 0, 0, 1, 1, 0, 0, -2, -1, 1, -1, 0, 0, 0, 1, 1, -1, 1, -1, 1, -1,
            1, 0, 1, 1, -2, 0, 4, -1, 0, 2, 1, 1, 1, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 1, 0, 1,
            1, -1, 0, 0, 0, 3, -1, 2, 0, -3, -1, 0, 1, 0, 0, -1, -1, 1, 1, 0, -2, 2, 1, 1, 0, 0, 0,
            0, 0, 0, 0, 0, -1, 0, 0, 0, -2, 1, 2, 2, 2, 2, -3, -1, 1, 1, 1, 0, -1, 1, 0, -1, 4, 1,
            -1, 0, 0, 0, 0, 1, 0, 1, 0, -1, 0, 1, 0, 1, 1, 2, 2, 1, 2, 2, 10, 0, 0, 0, 0, 1, 0, 1,
            -1, 1, 0, 0, 0, 0, 0, 0, 1, 1, 0, -1, 1, 0, 2, 1, -1, 1, 0, 0, 2, -2, -2, 11, -4, 1, 1,
            1, 1, 0, -1, -3, 2, -1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 0, 1, -1, -1, -1, 0, -1, 1, -2, 1,
            -2, 8, -8, -1, -1, 0, 1, 0, 0, -1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 0, 1, -1, 0, -1, 0, 0, 0,
            -1, 1, 1, 0, 9, 16, -8, 1, 1, 0, 1, 0, 1, -1, 0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0,
            0, -1, 0, 1, -1, 0, 0, 6, -7, -3, 0, 0, 0, 1, -1, -1, -1, 2, 2, 0, 1, 0, 1, 0, 1, 1, 1,
            0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 6, -9, 1, 1, -1, 1, 0, 0, 1, 0, 1, 1, 0, 0, -1, 0, 0,
            0, 0, 0, -1, 0, 0, 0, 0, 1, 1, 1, -2, 0, 0, 6, -5, 2, 2, 0, 1, 0, 0, 0, -1, 1, 1, 0, 0,
            0, 0, 0, 1, 0, 0, -1, 0, 1, -1, 0, 1, 0, 1, 1, 1, 1, 9, -9, 1, 1, 0, 1, 2, 1, 1, 1, 1,
            1, 0, 0, 0, 0, 0, -1, 0, 1, 0, 1, 1, 0, 0, 3, 1, 0, 1, -1, -2, 4, -9, 2, 0, 0, -1, 0,
            -1, 0, 0, 1, -1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1, -1, -2, 9, 6, 5, 0,
            0, -1, 0, 0, 0, 1, 0, 1, 0, 1, 1, 1, 1, -1, 1, -1, 0, 0, -1, 1, 1, 0, 0, -1, 1, 0, -1,
            10, -4, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0,
        ];

        assert_eq!(coefficients.len(), expected.len());

        let mut expected_i = expected.iter();
        let mut actual_i = coefficients.iter();

        for y in 0..code_block.rect.height() {
            for x in 0..code_block.rect.width() {
                let expected = expected_i.next().unwrap();
                let actual = actual_i.next().unwrap();
                assert_eq!(
                    expected, actual,
                    "x: {}, y: {}, expected: {}, actual: {}",
                    x, y, expected, actual
                );
            }
        }
    }
}
