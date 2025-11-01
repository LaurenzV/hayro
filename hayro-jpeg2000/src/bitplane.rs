//! Decoding bitplanes into sample coefficients.
//!
//! Some of the references are taken from the "JPEG2000 Standard for Image Compression" book
//! instead of the specification.

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::codestream::CodeBlockStyle;
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

    fn reset_for_next_bitplane(&mut self) {
        self.has_zero_coding = vec![0; self.has_zero_coding.len()];
    }

    fn significance_state(&self, position: &Position) -> u8 {
        self.significance_states[position.index(self.width)]
    }

    fn is_significant(&self, position: &Position) -> bool {
        self.significance_states[position.index(self.width)] != 0
    }

    fn set_significance_state(&mut self, position: &Position) {
        self.significance_states[position.index(self.width)] = 1;
    }

    fn set_has_zero_coding(&mut self, position: &Position) {
        self.has_zero_coding[position.index(self.width)] = 1;
    }

    fn set_has_magnitude_refinement(&mut self, position: &Position) {
        self.first_magnitude_refinement[position.index(self.width)] = 1;
    }

    fn is_magnitude_refined(&self, position: &Position) -> bool {
        self.first_magnitude_refinement[position.index(self.width)] != 0
    }

    fn has_zero_coding(&self, position: &Position) -> bool {
        self.has_zero_coding[position.index(self.width)] != 0
    }

    fn push_magnitude_bit(&mut self, position: &Position, bit: u16) {
        self.magnitude_array[position.index(self.width)].push_bit(bit);
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

    fn neighborhood_significances(&self, pos: &Position) -> u8 {
        self.horizontal_reference(pos) + self.vertical_reference(pos) + self.diagonal_reference(pos)
    }
}

pub(crate) fn decode(
    code_block: &mut CodeBlock,
    subband_type: SubbandType,
    style: &CodeBlockStyle,
) -> Option<()> {
    if code_block.number_of_coding_passes == 0 {
        return Some(());
    }

    if style.selective_arithmetic_coding_bypass
        || style.segmentation_symbols
        || style.vertically_causal_context
        || style.predictable_termination
        || style.termination_on_each_pass
        || style.reset_context_probabilities
    {
        unimplemented!();
    }

    let mut combined_layer_data: Vec<u8> = vec![];

    for data in &code_block.layer_data {
        combined_layer_data.extend(*data);
    }

    let combined_layers = code_block
        .layer_data
        .iter()
        .flat_map(|d| d.to_vec())
        .collect::<Vec<_>>();
    let mut decoder = ArithmeticDecoder::new(&combined_layers);

    decode_inner(code_block, subband_type, &mut decoder)
}

fn decode_inner(
    code_block: &mut CodeBlock,
    subband_type: SubbandType,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let mut ctx = BitplaneDecodeContext::new();
    ctx.reset(
        code_block.area.width(),
        code_block.area.height(),
        subband_type,
    );

    for coding_pass in 0..code_block.number_of_coding_passes {
        enum PassType {
            Cleanup,
            SignificancePropagation,
            MagnitudeRefinement,
        }

        let pass = match (coding_pass % 3) {
            0 => PassType::Cleanup,
            1 => PassType::SignificancePropagation,
            2 => PassType::MagnitudeRefinement,
            _ => unreachable!(),
        };

        match pass {
            PassType::Cleanup => {
                cleanup_pass(&mut ctx, decoder);
                ctx.reset_for_next_bitplane();
            }
            PassType::SignificancePropagation => {
                significance_propagation_pass(&mut ctx, decoder);
            }
            PassType::MagnitudeRefinement => {
                magnitude_refinement_pass(&mut ctx, decoder);
            }
        }
    }

    for (sign, magnitude) in ctx.signs.iter().zip(ctx.magnitude_array) {
        let mut num = magnitude.get() as i16;
        if *sign != 0 {
            num = -num;
        }
        code_block.coefficients.push(num);
    }

    Some(())
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

/// Perform the cleanup pass, specified in D.3.4.
/// See also the flow chart in Figure 7.3 in the JPEG2000 book.
fn cleanup_pass(ctx: &mut BitplaneDecodeContext, decoder: &mut impl BitDecoder) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(mut cur_pos) = position_iterator.next() else {
            break;
        };

        if !ctx.is_significant(&cur_pos) && !ctx.has_zero_coding(&cur_pos) {
            let use_rl = cur_pos.y % 4 == 0
                && (ctx.height - cur_pos.y) >= 4
                && ctx.neighborhood_significances(&cur_pos) == 0
                && ctx.neighborhood_significances(&Position::new(cur_pos.x, cur_pos.y + 1)) == 0
                && ctx.neighborhood_significances(&Position::new(cur_pos.x, cur_pos.y + 2)) == 0
                && ctx.neighborhood_significances(&Position::new(cur_pos.x, cur_pos.y + 3)) == 0;

            let bit = if use_rl {
                // "If the four contiguous coefficients in the column being scanned are all decoded
                // in the cleanup pass and the context label for all is 0 (including context
                // coefficients from previous magnitude, significance and cleanup passes), then the
                // unique run-length context is given to the arithmetic decoder along with the bit
                // stream. If the symbol 0 is returned, then all four contiguous coefficients in
                // the column remain insignificant and are set to zero.
                let bit = decoder.read_bit(ctx.ad_context(17));

                if bit == 0 {
                    ctx.push_magnitude_bit(&cur_pos, 0);

                    for _ in 0..3 {
                        cur_pos = position_iterator.next()?;
                        ctx.push_magnitude_bit(&cur_pos, 0);
                    }

                    continue;
                } else {
                    let mut num_zeroes = decoder.read_bit(ctx.ad_context(18));
                    num_zeroes = (num_zeroes << 1) | decoder.read_bit(ctx.ad_context(18));

                    for _ in 0..num_zeroes {
                        ctx.push_magnitude_bit(&cur_pos, 0);
                        cur_pos = position_iterator.next()?;
                    }

                    1
                }
            } else {
                let ctx_label = context_label_zero_coding(&cur_pos, &ctx);
                decoder.read_bit(ctx.ad_context(ctx_label))
            };

            ctx.push_magnitude_bit(&cur_pos, bit as u16);

            if bit == 1 {
                decode_sign_bit(&cur_pos, ctx, decoder);
                ctx.set_significance_state(&cur_pos);
            }
        }
    }

    Some(())
}

/// Section D.3.1.
/// See also the flow chart in Figure 7.4 in the JPEG2000 book.
fn significance_propagation_pass(
    ctx: &mut BitplaneDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(cur_pos) = position_iterator.next() else {
            break;
        };

        if !ctx.is_significant(&cur_pos) && ctx.neighborhood_significances(&cur_pos) != 0 {
            let ctx_label = context_label_zero_coding(&cur_pos, &ctx);
            let bit = decoder.read_bit(ctx.ad_context(ctx_label));
            ctx.push_magnitude_bit(&cur_pos, bit as u16);
            ctx.set_has_zero_coding(&cur_pos);

            if bit == 1 {
                decode_sign_bit(&cur_pos, ctx, decoder);
                ctx.set_significance_state(&cur_pos);
            }
        }
    }

    Some(())
}

/// Perform the magnitude refinement pass, specified in D.3.3.
/// See also the flow chart in Figure 7.5 in the JPEG2000 book.
fn magnitude_refinement_pass(
    ctx: &mut BitplaneDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let mut position_iterator = PositionIterator::new(ctx.width, ctx.height);

    loop {
        let Some(cur_pos) = position_iterator.next() else {
            break;
        };

        if ctx.is_significant(&cur_pos) && !ctx.has_zero_coding(&cur_pos) {
            let ctx_label = context_label_magnitude_refinement_coding(&cur_pos, &ctx);
            let bit = decoder.read_bit(ctx.ad_context(ctx_label));
            ctx.push_magnitude_bit(&cur_pos, bit as u16);
            ctx.set_has_magnitude_refinement(&cur_pos);
        }
    }

    Some(())
}

/// Section D.3.2.
fn decode_sign_bit(pos: &Position, ctx: &mut BitplaneDecodeContext, decoder: &mut impl BitDecoder) {
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

    let (ctx_label, xor_bit) = context_label_sign_coding(&pos, ctx);
    let ad_ctx = ctx.ad_context(ctx_label);
    let sign_bit = decoder.read_bit(ad_ctx) ^ xor_bit as u32;
    ctx.set_sign(pos, sign_bit as u8);
}

/// Section D.3.1.
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
    inner: u16,
    count: u8,
}

impl ComponentBitPlanes {
    fn push_bit(&mut self, bit: u16) {
        assert!(self.count < 16);
        assert!(bit < 2);

        self.inner = (self.inner << 1) | bit;
        self.count += 1;
    }

    fn get(&self) -> u16 {
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
    use super::{BitDecoder, PositionIterator, decode, decode_inner};
    use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
    use crate::codestream::CodeBlockStyle;
    use crate::packet::{CodeBlock, SubbandType};
    use crate::tile::IntRect;
    use hayro_common::bit::{BitReader, BitWriter};

    struct DummyBitDecoder<'a>(BitReader<'a>);

    impl BitDecoder for DummyBitDecoder<'_> {
        fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
            self.0.read(1).unwrap()
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

    /// Example 7.3.2 in the JPEG2000 book.
    #[test]
    fn bitplane_decoding_1() {
        let data = {
            let mut buf = vec![0; 8];
            let mut writer = BitWriter::new(&mut buf, 1).unwrap();

            // CUP bitplane 2.
            writer.write_bits([
                1, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
            ]);

            // SPP bitplane 1.
            writer.write_bits([1, 0, 1, 1, 0, 0, 0, 0, 1, 0, 1, 1, 0, 0, 1, 1, 1, 0]);

            // MRP bitplane 1.
            writer.write_bits([0, 1, 1, 0]);

            // No CUP for bitplane 1.

            // SPP for bitplane 0.
            writer.write_bits([0, 0, 1, 0, 0, 0, 1, 0]);

            // MRP for bitplane 0.
            writer.write_bits([1, 1, 0, 1, 0, 0, 0, 1, 1, 0]);

            // No CUP for bitplane 0.

            buf
        };

        let bit_reader = BitReader::new(&data);
        let mut decoder = DummyBitDecoder(bit_reader);

        let mut code_block = CodeBlock {
            area: IntRect::from_xywh(0, 0, 4, 4),
            x_idx: 0,
            y_idx: 0,
            layer_data: vec![&data],
            has_been_included: false,
            missing_bit_planes: 0,
            number_of_coding_passes: 7,
            l_block: 0,
            coefficients: vec![],
        };

        decode_inner(&mut code_block, SubbandType::LowLow, &mut decoder);

        assert_eq!(
            code_block.coefficients,
            vec![3, 0, 0, 5, -3, 7, 2, 1, -4, -1, -2, 3, 0, 6, 0, 2]
        );
    }

    // First packet from example in Section J.10.4.
    #[test]
    fn bitplane_decoding_2() {
        let data = vec![0x01, 0x8f, 0x0d, 0xc8, 0x75, 0x5d];

        let bit_reader = BitReader::new(&data);

        let mut code_block = CodeBlock {
            area: IntRect::from_xywh(0, 0, 1, 5),
            x_idx: 0,
            y_idx: 0,
            layer_data: vec![&data],
            has_been_included: false,
            missing_bit_planes: 0,
            number_of_coding_passes: 16,
            l_block: 0,
            coefficients: vec![],
        };

        decode(
            &mut code_block,
            SubbandType::LowLow,
            &CodeBlockStyle::default(),
        );

        assert_eq!(code_block.coefficients, vec![-26, -22, -30, -32, -19]);
    }

    // Second packet from example in Section J.10.4.
    #[test]
    fn bitplane_decoding_3() {
        let data = vec![0x0F, 0xB1, 0x76];

        let bit_reader = BitReader::new(&data);

        let mut code_block = CodeBlock {
            area: IntRect::from_xywh(0, 0, 1, 4),
            x_idx: 0,
            y_idx: 0,
            layer_data: vec![&data],
            has_been_included: false,
            missing_bit_planes: 0,
            number_of_coding_passes: 7,
            l_block: 0,
            coefficients: vec![],
        };

        decode(
            &mut code_block,
            SubbandType::LowHigh,
            &CodeBlockStyle::default(),
        );

        assert_eq!(code_block.coefficients, vec![1, 5, 1, 0]);
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

        let bit_reader = BitReader::new(&data);

        let mut code_block = CodeBlock {
            area: IntRect::from_xywh(0, 0, 32, 32),
            x_idx: 0,
            y_idx: 0,
            layer_data: vec![&data],
            has_been_included: false,
            missing_bit_planes: 5,
            number_of_coding_passes: 13,
            l_block: 0,
            coefficients: vec![],
        };

        decode(
            &mut code_block,
            SubbandType::HighLow,
            &CodeBlockStyle::default(),
        );

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

        assert_eq!(code_block.coefficients.len(), expected.len());

        let mut expected_i = expected.iter();
        let mut actual_i = code_block.coefficients.iter();

        for y in 0..code_block.area.height() {
            for x in 0..code_block.area.width() {
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
