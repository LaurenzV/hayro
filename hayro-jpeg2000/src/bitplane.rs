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
use crate::decode::{CodeBlock, DecompositionStorage, SubBandType};
use crate::reader::BitReader;

#[derive(Default)]
pub(crate) struct BitPlaneDecodeBuffers {
    combined_layers: Vec<u8>,
    segment_ranges: Vec<usize>,
    segment_coding_passes: Vec<u8>,
}
impl BitPlaneDecodeBuffers {
    fn reset(&mut self) {
        self.combined_layers.clear();
        self.segment_ranges.clear();
        self.segment_coding_passes.clear();

        // The design of these two buffers is that the ranges are stored
        // as [idx, idx + 1), so we need to store the first 0 when resetting.
        self.segment_ranges.push(0);
        self.segment_coding_passes.push(0);
    }
}

/// Decode the layers of the given code block into coefficients.
///
/// The result will be stored in the form of a vector of signs and magnitudes
/// in the bitplane decoder context.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode(
    code_block: &CodeBlock,
    sub_band_type: SubBandType,
    mut num_bitplanes: u8,
    style: &CodeBlockStyle,
    ctx: &mut CodeBlockDecodeContext,
    bp_buffers: &mut BitPlaneDecodeBuffers,
    storage: &DecompositionStorage,
    strict: bool,
) -> Result<(), &'static str> {
    ctx.reset(code_block, sub_band_type, style);

    if code_block.number_of_coding_passes == 0 {
        return Ok(());
    }

    // "The maximum number of bit-planes available for the representation of
    // coefficients in any sub-band, b, is given by Mb as defined in Equation
    // (E-2). In general however, the number of actual bit-planes for which
    // coding passes are generated is Mb – P, where the number of missing most
    // significant bit-planes, P, may vary from code-block to code-block."

    // See issue 399. If this subtraction fails the file is in theory invalid,
    // but we still try to be lenient.
    num_bitplanes = if strict {
        num_bitplanes
            .checked_sub(code_block.missing_bit_planes)
            .ok_or("number of missing bit planes was too hgh")?
    } else {
        num_bitplanes.saturating_sub(code_block.missing_bit_planes)
    };

    if num_bitplanes == 0 {
        return Ok(());
    }

    let max_coding_passes = if num_bitplanes == 1 {
        1
    } else {
        1 + 3 * (num_bitplanes - 1)
    };

    if max_coding_passes < code_block.number_of_coding_passes && strict {
        return Err("codeblock contains too many coding passes");
    }

    decode_inner(
        code_block,
        num_bitplanes,
        max_coding_passes,
        storage,
        ctx,
        bp_buffers,
        strict,
    )
    .ok_or("failed to decode code-block")?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_inner(
    code_block: &CodeBlock,
    num_bitplanes: u8,
    max_coding_passes: u8,
    storage: &DecompositionStorage,
    ctx: &mut CodeBlockDecodeContext,
    bp_buffers: &mut BitPlaneDecodeBuffers,
    strict: bool,
) -> Option<()> {
    bp_buffers.reset();

    let mut last_segment_idx = 0;
    let mut coding_passes = 0;

    for layer in &storage.layers[code_block.layers.start..code_block.layers.end] {
        if let Some(range) = layer.segments.clone() {
            let layer_segments = &storage.segments[range.clone()];
            for segment in layer_segments {
                if segment.idx != last_segment_idx {
                    assert_eq!(segment.idx, last_segment_idx + 1);

                    bp_buffers
                        .segment_ranges
                        .push(bp_buffers.combined_layers.len());
                    bp_buffers.segment_coding_passes.push(coding_passes);
                    last_segment_idx += 1;
                }

                bp_buffers.combined_layers.extend(segment.data);
                coding_passes += segment.coding_pases;
            }
        }
    }

    assert_eq!(coding_passes, code_block.number_of_coding_passes);

    bp_buffers
        .segment_ranges
        .push(bp_buffers.combined_layers.len());
    bp_buffers.segment_coding_passes.push(coding_passes);

    let is_normal_mode =
        !ctx.style.selective_arithmetic_coding_bypass && !ctx.style.termination_on_each_pass;

    if is_normal_mode {
        // Only one termination per code block, so we can just decode the
        // whole range in one single pass.
        let mut decoder = ArithmeticDecoder::new(&bp_buffers.combined_layers);
        handle_coding_passes(
            0,
            code_block.number_of_coding_passes.min(max_coding_passes),
            ctx,
            &mut decoder,
            strict,
        )?;
    } else {
        // Otherwise, each segment introduces a termination. For selective
        // arithmetic coding bypass, each segment only covers one coding pass
        // and a termination is introduced every time. Otherwise, for only
        // arithmetic coding bypass, terminations are introduced based on the
        // exact index of the covered coding passes (see Table D.9).
        for segment in 0..bp_buffers.segment_coding_passes.len() - 1 {
            let start_coding_pass = bp_buffers.segment_coding_passes[segment];
            let end_coding_pass =
                bp_buffers.segment_coding_passes[segment + 1].min(max_coding_passes);

            let data = &bp_buffers.combined_layers
                [bp_buffers.segment_ranges[segment]..bp_buffers.segment_ranges[segment + 1]];

            let use_arithmetic = if ctx.style.selective_arithmetic_coding_bypass {
                if start_coding_pass <= 9 {
                    true
                } else {
                    // Only for cleanup pass.
                    start_coding_pass.is_multiple_of(3)
                }
            } else {
                true
            };

            if use_arithmetic {
                let mut decoder = ArithmeticDecoder::new(data);
                handle_coding_passes(
                    start_coding_pass,
                    end_coding_pass,
                    ctx,
                    &mut decoder,
                    strict,
                )?;
            } else {
                let mut decoder = BypassDecoder::new(data, strict);
                handle_coding_passes(
                    start_coding_pass,
                    end_coding_pass,
                    ctx,
                    &mut decoder,
                    strict,
                )?;
            }
        }
    }

    // Extend all coefficients with zero bits until we have the required number
    // of bits.
    for (coefficient, coefficient_state) in ctx
        .coefficients
        .iter_mut()
        .zip(ctx.coefficient_states.iter().copied())
    {
        let count = num_bitplanes - coefficient_state.num_bitplanes();
        coefficient.push_zeroes(count);
    }

    Some(())
}

fn handle_coding_passes(
    start: u8,
    end: u8,
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
    strict: bool,
) -> Option<()> {
    for coding_pass in start..end {
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
                cleanup_pass(ctx, decoder)?;

                if ctx.style.segmentation_symbols {
                    let b0 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b1 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b2 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b3 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;

                    if (b0 != 1 || b1 != 0 || b2 != 1 || b3 != 0) && strict {
                        return None;
                    }
                }

                ctx.reset_for_next_bitplane();
            }
            PassType::SignificancePropagation => {
                significance_propagation_pass(ctx, decoder)?;
            }
            PassType::MagnitudeRefinement => {
                magnitude_refinement_pass(ctx, decoder)?;
            }
        }

        if ctx.style.reset_context_probabilities {
            ctx.reset_contexts();
        }
    }

    Some(())
}

// We only allow 31 bit planes because we need one bit for the sign.
pub(crate) const BITPLANE_BIT_SIZE: u32 = size_of::<u32>() as u32 * 8 - 1;

const SIGNIFICANCE_SHIFT: u8 = 7;
const HAS_MAGNITUDE_REFINEMENT_SHIFT: u8 = 6;
const HAS_ZERO_CODING_SHIFT: u8 = 5;
const BITPLANE_COUNT_MASK: u8 = (1 << 5) - 1;

/// From MSB to LSB:
/// Bit 1 represents the significance state of each coefficient. Will be
/// set to one as soon as the first non-zero bit for that coefficient is
/// encountered.
/// Bit 2 stores whether the coefficient has previously had (at least one)
/// magnitude refinement pass.
/// Bit 3 stores whether the given coefficient belongs to a zero coding pass
/// applied as part of sign propagation in the current bitplane. This
/// value will be reset every time we advance to a new bitplane.
/// Bits 4-8 store the current number of bitplanes for the given coefficient.
/// Five bits are enough to store 0-31, which works out nicely because our
/// maximum number of bitplanes also is 31.
#[derive(Default, Copy, Clone)]
pub(crate) struct CoefficientState(u8);

impl CoefficientState {
    #[inline(always)]
    fn set_bit(&mut self, shift: u8, value: u8) {
        debug_assert!(value < 2);

        self.0 &= !(1u8 << shift);
        self.0 |= value << shift;
    }

    #[inline(always)]
    fn set_significant(&mut self) {
        self.set_bit(SIGNIFICANCE_SHIFT, 1);
    }

    #[inline(always)]
    fn set_zero_coded(&mut self, value: u8) {
        self.set_bit(HAS_ZERO_CODING_SHIFT, value & 1);
    }

    #[inline(always)]
    fn set_magnitude_refined(&mut self) {
        self.set_bit(HAS_MAGNITUDE_REFINEMENT_SHIFT, 1);
    }

    #[inline(always)]
    fn is_significant(&self) -> bool {
        self.significance() == 1
    }

    #[inline(always)]
    fn significance(&self) -> u8 {
        (self.0 >> SIGNIFICANCE_SHIFT) & 1
    }

    #[inline(always)]
    fn magnitude_refinement(&self) -> u8 {
        (self.0 >> HAS_MAGNITUDE_REFINEMENT_SHIFT) & 1
    }

    #[inline(always)]
    fn is_zero_coded(&self) -> bool {
        (self.0 >> HAS_ZERO_CODING_SHIFT) & 1 == 1
    }

    #[inline(always)]
    fn num_bitplanes(&self) -> u8 {
        self.0 & BITPLANE_COUNT_MASK
    }

    #[inline(always)]
    fn set_magnitude_bits(&mut self, count: u8) {
        debug_assert!((count as u32) <= BITPLANE_BIT_SIZE);
        self.0 = (self.0 & !BITPLANE_COUNT_MASK) | (count & BITPLANE_COUNT_MASK);
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Coefficient(u32);

impl Coefficient {
    pub(crate) fn get(&self) -> i32 {
        let mut magnitude = (self.0 & !0x80000000) as i32;
        // Map sign (0 for positive, 1 for negative) to 1, -1.
        magnitude *= 1 - 2 * (self.sign() as i32);

        magnitude
    }

    fn set_sign(&mut self, sign: u8) {
        self.0 |= (sign as u32) << 31;
    }

    fn has_sign(&self) -> bool {
        self.0 & 0x80000000 != 0
    }

    fn sign(&self) -> u32 {
        (self.0 >> 31) & 1
    }

    fn push_bit(&mut self, bit: u32) {
        let sign = self.0 & 0x80000000;
        self.0 = sign | ((self.0 << 1) | bit);
    }

    fn push_zeroes(&mut self, num: u8) {
        let sign = self.0 & 0x80000000;
        self.0 = sign | self.0 << num;
    }
}

const COEFFICIENTS_PADDING: u32 = 1;

/// Store the significances of each neighbor for a specific coefficient.
/// The order from MSB to LSB is as follows:
///
/// top-left, top, top-right, left, bottom-left, right, bottom-right, bottom.
///
/// See the `context_label_sign_coding` method for why we aren't simply using
/// row-major order.
#[derive(Default, Copy, Clone)]
struct NeighborSignificances(u8);

impl NeighborSignificances {
    fn set_top_left(&mut self) {
        self.0 |= 1 << 7;
    }

    fn set_top(&mut self) {
        self.0 |= 1 << 6;
    }

    fn set_top_right(&mut self) {
        self.0 |= 1 << 5;
    }

    fn set_left(&mut self) {
        self.0 |= 1 << 4;
    }

    fn set_bottom_left(&mut self) {
        self.0 |= 1 << 3;
    }

    fn set_right(&mut self) {
        self.0 |= 1 << 2;
    }

    fn set_bottom_right(&mut self) {
        self.0 |= 1 << 1;
    }

    fn set_bottom(&mut self) {
        self.0 |= 1;
    }

    fn all(&self) -> u8 {
        self.0
    }

    fn all_without_bottom(&self) -> u8 {
        self.0 & 0b11110100
    }
}

pub(crate) struct CodeBlockDecodeContext {
    /// A vector of bit-packed fields for each coefficient in the code-block.
    coefficient_states: Vec<CoefficientState>,
    /// The neighbor significances for each coefficient.
    neighbor_significances: Vec<NeighborSignificances>,
    /// The magnitude and signs each coefficient that is successively built
    /// as we advance through the bitplanes.
    coefficients: Vec<Coefficient>,
    /// The width of the code-block we are processing.
    width: u32,
    /// The width of the code-block we are processing, with padding.
    padded_width: u32,
    /// The height of the code-block we are processing.
    height: u32,
    /// The code-block style for the current code-block.
    style: CodeBlockStyle,
    /// The type of sub-band the current code block belongs to.
    sub_band_type: SubBandType,
    /// The arithmetic decoder contexts for each context label.
    contexts: [ArithmeticDecoderContext; 19],
}

impl Default for CodeBlockDecodeContext {
    fn default() -> Self {
        Self {
            coefficient_states: vec![],
            coefficients: vec![],
            neighbor_significances: vec![],
            width: 0,
            padded_width: COEFFICIENTS_PADDING * 2,
            height: 0,
            style: CodeBlockStyle::default(),
            sub_band_type: SubBandType::LowLow,
            contexts: [ArithmeticDecoderContext::default(); 19],
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
        let padded_width = width + COEFFICIENTS_PADDING * 2;
        let padded_height = height + COEFFICIENTS_PADDING * 2;
        let num_coefficients = padded_width as usize * padded_height as usize;

        self.coefficients.clear();
        self.coefficients
            .resize(num_coefficients, Coefficient::default());

        self.neighbor_significances.clear();
        self.neighbor_significances
            .resize(num_coefficients, NeighborSignificances::default());

        self.coefficient_states.clear();
        self.coefficient_states
            .resize(num_coefficients, CoefficientState::default());

        self.width = width;
        self.padded_width = padded_width;
        self.height = height;
        self.sub_band_type = sub_band_type;
        self.style = *code_block_style;
        self.reset_contexts();
    }

    pub(crate) fn coefficient_rows(&self) -> impl Iterator<Item = &[Coefficient]> {
        self.coefficients
            .chunks_exact(self.padded_width as usize)
            .map(|row| &row[COEFFICIENTS_PADDING as usize..][..self.width as usize])
            .skip(COEFFICIENTS_PADDING as usize)
            .take(self.height as usize)
    }

    fn set_sign(&mut self, pos: Position, sign: u8) {
        // Using `or` is okay here because we only set the sign once.
        self.coefficients[pos.index(self.padded_width)].set_sign(sign);
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
        for el in &mut self.coefficient_states {
            el.set_zero_coded(0);
        }
    }

    fn is_significant(&self, position: Position) -> bool {
        self.coefficient_states[position.index(self.padded_width)].is_significant()
    }

    fn set_significant(&mut self, position: Position) {
        let idx = position.index(self.padded_width);
        let is_significant = self.coefficient_states[idx].is_significant();

        if !is_significant {
            self.coefficient_states[idx].set_significant();

            // Update all neighbors so they know this coefficient is significant
            // now.
            self.neighbor_significances[position.top_left().index(self.padded_width)]
                .set_bottom_right();
            self.neighbor_significances[position.top().index(self.padded_width)].set_bottom();
            self.neighbor_significances[position.top_right().index(self.padded_width)]
                .set_bottom_left();
            self.neighbor_significances[position.left().index(self.padded_width)].set_right();
            self.neighbor_significances[position.right().index(self.padded_width)].set_left();
            self.neighbor_significances[position.bottom_left().index(self.padded_width)]
                .set_top_right();
            self.neighbor_significances[position.bottom().index(self.padded_width)].set_top();
            self.neighbor_significances[position.bottom_right().index(self.padded_width)]
                .set_top_left();
        }
    }

    fn set_zero_coded(&mut self, position: Position) {
        self.coefficient_states[position.index(self.padded_width)].set_zero_coded(1);
    }

    fn set_magnitude_refined(&mut self, position: Position) {
        self.coefficient_states[position.index(self.padded_width)].set_magnitude_refined();
    }

    fn magnitude_refinement(&self, position: Position) -> u8 {
        self.coefficient_states[position.index(self.padded_width)].magnitude_refinement()
    }

    fn is_zero_coded(&self, position: Position) -> bool {
        self.coefficient_states[position.index(self.padded_width)].is_zero_coded()
    }

    fn push_magnitude_bit(&mut self, position: Position, bit: u32) {
        let idx = position.index(self.padded_width);
        let count = self.coefficient_states[idx].num_bitplanes();

        debug_assert!((count as u32) < BITPLANE_BIT_SIZE);

        self.coefficients[idx].push_bit(bit);
        self.coefficient_states[idx].set_magnitude_bits(count + 1);
    }

    #[inline]
    fn sign(&self, position: Position) -> u8 {
        if self.coefficients[position.index(self.padded_width)].has_sign() {
            1
        } else {
            0
        }
    }

    #[inline]
    fn neighbor_in_next_stripe(&self, pos: Position, neighbor_y: u32) -> bool {
        neighbor_y < self.height && (neighbor_y >> 2) > (pos.real_y() >> 2)
    }

    #[inline]
    fn neighborhood_significance_states(&self, pos: Position) -> u8 {
        let neighbors = &self.neighbor_significances[pos.index(self.padded_width)];

        if self.style.vertically_causal_context
            && self.neighbor_in_next_stripe(pos, pos.real_y() + 1)
        {
            neighbors.all_without_bottom()
        } else {
            neighbors.all()
        }
    }
}

/// Perform the cleanup pass, specified in D.3.4.
/// See also the flow chart in Figure 7.3 in the JPEG2000 book.
fn cleanup_pass(ctx: &mut CodeBlockDecodeContext, decoder: &mut impl BitDecoder) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            if !ctx.is_significant(*cur_pos) && !ctx.is_zero_coded(*cur_pos) {
                let use_rl = cur_pos.real_y() % 4 == 0
                    && (ctx.height - cur_pos.real_y()) >= 4
                    && ctx.neighborhood_significance_states(*cur_pos) == 0
                    && ctx.neighborhood_significance_states(Position::new_index(
                        cur_pos.index_x,
                        cur_pos.index_y + 1,
                    )) == 0
                    && ctx.neighborhood_significance_states(Position::new_index(
                        cur_pos.index_x,
                        cur_pos.index_y + 2,
                    )) == 0
                    && ctx.neighborhood_significance_states(Position::new_index(
                        cur_pos.index_x,
                        cur_pos.index_y + 3,
                    )) == 0;

                let bit = if use_rl {
                    // "If the four contiguous coefficients in the column being scanned are all decoded
                    // in the cleanup pass and the context label for all is 0 (including context
                    // coefficients from previous magnitude, significance and cleanup passes), then the
                    // unique run-length context is given to the arithmetic decoder along with the bit
                    // stream."
                    let bit = decoder.read_bit(ctx.arithmetic_decoder_context(17))?;

                    if bit == 0 {
                        // "If the symbol 0 is returned, then all four contiguous coefficients in
                        // the column remain insignificant and are set to zero."
                        ctx.push_magnitude_bit(*cur_pos, 0);

                        for _ in 0..3 {
                            cur_pos.index_y += 1;
                            ctx.push_magnitude_bit(*cur_pos, 0);
                        }

                        return Some(());
                    } else {
                        // "Otherwise, if the symbol 1 is returned, then at least
                        // one of the four contiguous coefficients in the column is
                        // significant. The next two bits, returned with the
                        // UNIFORM context (index 46 in Table C.2), denote which
                        // coefficient from the top of the column down is the first
                        // to be found significant."
                        let mut num_zeroes =
                            decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                        num_zeroes = (num_zeroes << 1)
                            | decoder.read_bit(ctx.arithmetic_decoder_context(18))?;

                        for _ in 0..num_zeroes {
                            ctx.push_magnitude_bit(*cur_pos, 0);
                            cur_pos.index_y += 1;
                        }

                        1
                    }
                } else {
                    let ctx_label = context_label_zero_coding(*cur_pos, ctx);
                    decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?
                };

                ctx.push_magnitude_bit(*cur_pos, bit);

                if bit == 1 {
                    decode_sign_bit(*cur_pos, ctx, decoder);
                    ctx.set_significant(*cur_pos);
                }
            }

            Some(())
        },
    )
}

/// Perform the significance propagation pass (Section D.3.1).
///
/// See also the flow chart in Figure 7.4 in the JPEG2000 book.
fn significance_propagation_pass(
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            // "The significance propagation pass only includes bits of coefficients
            // that were insignificant (the significance state has yet to be set)
            // and have a non-zero context."
            if !ctx.is_significant(*cur_pos) && ctx.neighborhood_significance_states(*cur_pos) != 0
            {
                let ctx_label = context_label_zero_coding(*cur_pos, ctx);
                let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?;
                ctx.push_magnitude_bit(*cur_pos, bit);
                ctx.set_zero_coded(*cur_pos);

                // "If the value of this bit is 1 then the significance
                // state is set to 1 and the immediate next bit to be decoded is
                // the sign bit for the coefficient. Otherwise, the significance
                // state remains 0."
                if bit == 1 {
                    decode_sign_bit(*cur_pos, ctx, decoder)?;
                    ctx.set_significant(*cur_pos);
                }
            }

            Some(())
        },
    )
}

/// Perform the magnitude refinement pass, specified in Section D.3.3.
///
/// See also the flow chart in Figure 7.5 in the JPEG2000 book.
fn magnitude_refinement_pass(
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            if ctx.is_significant(*cur_pos) && !ctx.is_zero_coded(*cur_pos) {
                let ctx_label = context_label_magnitude_refinement_coding(*cur_pos, ctx);
                let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?;
                ctx.push_magnitude_bit(*cur_pos, bit);
                ctx.set_magnitude_refined(*cur_pos);
            }

            Some(())
        },
    )
}

fn for_each_position(
    width: u32,
    height: u32,
    mut action: impl FnMut(&mut Position) -> Option<()>,
) -> Option<()> {
    // "Each bit-plane of a code-block is scanned in a particular order.
    // Starting at the top left, the first four coefficients of the
    // first column are scanned, followed by the first four coefficients of
    // the second column and so on, until the right side of the code-block
    // is reached. The scan then returns to the left of the code-block and
    // the second set of four coefficients in each column is scanned. The
    // process is continued to the bottom of the code-block. If the
    // code-block height is not divisible by 4, the last set of coefficients
    // scanned in each column will contain fewer than 4 members."
    for base_row in (0..height).step_by(4) {
        for x in 0..width {
            let mut cur_pos = Position::new(x, base_row);
            while cur_pos.real_y() < (base_row + 4).min(height) {
                action(&mut cur_pos)?;
                cur_pos.index_y += 1;
            }
        }
    }

    Some(())
}

/// See `context_label_sign_coding`. This table contains all context labels
/// for each combination of the bit-packed field. (255, 255) represent
/// impossible combinations.
#[rustfmt::skip]
const SIGN_CONTEXT_LOOKUP: [(u8, u8); 256] = [
    (9,0), (10,0), (10,1), (0,0), (12,0), (13,0), (11,0), (0,0), (12,1), (11,1), (13,1), (0,0), (0,0), (0,0), (0,0), (0,0), (12,0), (13,0), (11,0), (0,0), (12,0), (13,0), (11,0), (0,0), (9,0),
    (10,0), (10,1), (0,0), (0,0), (0,0), (0,0), (0,0), (12,1), (11,1), (13,1), (0,0), (9,0), (10,0), (10,1), (0,0), (12,1), (11,1), (13,1), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (10,0), (10,0), (9,0), (0,0), (13,0), (13,0), (12,0), (0,0), (11,1), (11,1), (12,1),
    (0,0), (0,0), (0,0), (0,0), (0,0), (13,0), (13,0), (12,0), (0,0), (13,0), (13,0), (12,0), (0,0), (10,0), (10,0), (9,0), (0,0), (0,0), (0,0), (0,0), (0,0), (11,1), (11,1), (12,1), (0,0),
    (10,0), (10,0), (9,0), (0,0), (11,1), (11,1), (12,1), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (10,1), (9,0), (10,1), (0,0), (11,0), (12,0), (11,0), (0,0), (13,1), (12,1), (13,1), (0,0), (0,0), (0,0), (0,0), (0,0), (11,0), (12,0), (11,0), (0,0), (11,0), (12,0),
    (11,0), (0,0), (10,1), (9,0), (10,1), (0,0), (0,0), (0,0), (0,0), (0,0), (13,1), (12,1), (13,1), (0,0), (10,1), (9,0), (10,1), (0,0), (13,1), (12,1), (13,1), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
];

#[rustfmt::skip]
const ZERO_CTX_LL_LH_LOOKUP: [u8; 256] = [
    0, 3, 1, 3, 5, 7, 6, 7, 1, 3, 2, 3, 6, 7, 6, 7, 5, 7, 6, 7, 8, 8, 8, 8, 6,
    7, 6, 7, 8, 8, 8, 8, 1, 3, 2, 3, 6, 7, 6, 7, 2, 3, 2, 3, 6, 7, 6, 7, 6, 7,
    6, 7, 8, 8, 8, 8, 6, 7, 6, 7, 8, 8, 8, 8, 3, 4, 3, 4, 7, 7, 7, 7, 3, 4, 3,
    4, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 7, 7, 7, 7, 8, 8, 8, 8, 3, 4, 3, 4,
    7, 7, 7, 7, 3, 4, 3, 4, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 7, 7, 7, 7, 8,
    8, 8, 8, 1, 3, 2, 3, 6, 7, 6, 7, 2, 3, 2, 3, 6, 7, 6, 7, 6, 7, 6, 7, 8, 8,
    8, 8, 6, 7, 6, 7, 8, 8, 8, 8, 2, 3, 2, 3, 6, 7, 6, 7, 2, 3, 2, 3, 6, 7, 6,
    7, 6, 7, 6, 7, 8, 8, 8, 8, 6, 7, 6, 7, 8, 8, 8, 8, 3, 4, 3, 4, 7, 7, 7, 7,
    3, 4, 3, 4, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 7, 7, 7, 7, 8, 8, 8, 8, 3,
    4, 3, 4, 7, 7, 7, 7, 3, 4, 3, 4, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 7, 7,
    7, 7, 8, 8, 8, 8,
];

#[rustfmt::skip]
const ZERO_CTX_HL_LOOKUP: [u8; 256] = [
    0, 5, 1, 6, 3, 7, 3, 7, 1, 6, 2, 6, 3, 7, 3, 7, 3, 7, 3, 7, 4, 7, 4, 7, 3,
    7, 3, 7, 4, 7, 4, 7, 1, 6, 2, 6, 3, 7, 3, 7, 2, 6, 2, 6, 3, 7, 3, 7, 3, 7,
    3, 7, 4, 7, 4, 7, 3, 7, 3, 7, 4, 7, 4, 7, 5, 8, 6, 8, 7, 8, 7, 8, 6, 8, 6,
    8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 6, 8, 6, 8,
    7, 8, 7, 8, 6, 8, 6, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7,
    8, 7, 8, 1, 6, 2, 6, 3, 7, 3, 7, 2, 6, 2, 6, 3, 7, 3, 7, 3, 7, 3, 7, 4, 7,
    4, 7, 3, 7, 3, 7, 4, 7, 4, 7, 2, 6, 2, 6, 3, 7, 3, 7, 2, 6, 2, 6, 3, 7, 3,
    7, 3, 7, 3, 7, 4, 7, 4, 7, 3, 7, 3, 7, 4, 7, 4, 7, 6, 8, 6, 8, 7, 8, 7, 8,
    6, 8, 6, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 6,
    8, 6, 8, 7, 8, 7, 8, 6, 8, 6, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8,
    7, 8, 7, 8, 7, 8,
];

#[rustfmt::skip]
const ZERO_CTX_HH_LOOKUP: [u8; 256] = [
    0, 1, 3, 4, 1, 2, 4, 5, 3, 4, 6, 7, 4, 5, 7, 7, 1, 2, 4, 5, 2, 2, 5, 5, 4,
    5, 7, 7, 5, 5, 7, 7, 3, 4, 6, 7, 4, 5, 7, 7, 6, 7, 8, 8, 7, 7, 8, 8, 4, 5,
    7, 7, 5, 5, 7, 7, 7, 7, 8, 8, 7, 7, 8, 8, 1, 2, 4, 5, 2, 2, 5, 5, 4, 5, 7,
    7, 5, 5, 7, 7, 2, 2, 5, 5, 2, 2, 5, 5, 5, 5, 7, 7, 5, 5, 7, 7, 4, 5, 7, 7,
    5, 5, 7, 7, 7, 7, 8, 8, 7, 7, 8, 8, 5, 5, 7, 7, 5, 5, 7, 7, 7, 7, 8, 8, 7,
    7, 8, 8, 3, 4, 6, 7, 4, 5, 7, 7, 6, 7, 8, 8, 7, 7, 8, 8, 4, 5, 7, 7, 5, 5,
    7, 7, 7, 7, 8, 8, 7, 7, 8, 8, 6, 7, 8, 8, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 7, 7, 8, 8, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 4, 5, 7, 7, 5, 5, 7, 7,
    7, 7, 8, 8, 7, 7, 8, 8, 5, 5, 7, 7, 5, 5, 7, 7, 7, 7, 8, 8, 7, 7, 8, 8, 7,
    7, 8, 8, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 7, 7, 8, 8, 7, 7, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8,
];

/// Decode a sign bit (Section D.3.2).
#[inline(always)]
fn decode_sign_bit<T: BitDecoder>(
    pos: Position,
    ctx: &mut CodeBlockDecodeContext,
    decoder: &mut T,
) -> Option<()> {
    /// Based on Table D.2.
    #[inline(always)]
    fn context_label_sign_coding(pos: Position, ctx: &CodeBlockDecodeContext) -> (u8, u8) {
        // A lot of subtleties going on here, all in the interest of achieving
        // the best performance. Fundamentally, we need to determine the
        // significances as well as signs of the four neighbors (i.e. not
        // including the diagonal neighbors) and based on what the sum of signs
        // is, we assign a context label.
        let suppress_lower = ctx.style.vertically_causal_context
            && ctx.neighbor_in_next_stripe(pos, pos.real_y() + 1);
        // First, let's get all neighbor significances and mask out the diagonals.
        let significances = ctx.neighborhood_significance_states(pos) & 0b0101_0101;

        // Get all the signs.
        let left_sign = ctx.sign(pos.left());
        let right_sign = ctx.sign(pos.right());
        let top_sign = ctx.sign(pos.top());
        let bottom_sign = if suppress_lower {
            0
        } else {
            ctx.sign(pos.bottom())
        };

        // Due to the specific layout of `NeighborSignificances`, direct neighbors
        // and diagonals are interleaved. Therefore, we create a new bit-packed
        // representation that indicates whether the top/left/right/bottom sign
        // is positive, negative, or insignificant. We need two bits for this.
        // 00 represents insignificant, 01 positive and 10 negative. 11
        // is an invalid combination.
        let signs = (top_sign << 6) | (left_sign << 4) | (right_sign << 2) | bottom_sign;
        let negative_significances = significances & signs;
        let positive_significances = significances & !signs;
        let merged_significances = (negative_significances << 1) | positive_significances;

        SIGN_CONTEXT_LOOKUP[merged_significances as usize]
    }

    let (ctx_label, xor_bit) = context_label_sign_coding(pos, ctx);
    let ad_ctx = ctx.arithmetic_decoder_context(ctx_label);
    let sign_bit = if T::IS_BYPASS {
        decoder.read_bit(ad_ctx)?
    } else {
        decoder.read_bit(ad_ctx)? ^ xor_bit as u32
    };
    ctx.set_sign(pos, sign_bit as u8);

    Some(())
}

/// Return the context label for zero coding (Section D.3.1).
#[inline(always)]
fn context_label_zero_coding(pos: Position, ctx: &CodeBlockDecodeContext) -> u8 {
    let neighbors = ctx.neighborhood_significance_states(pos);

    match ctx.sub_band_type {
        SubBandType::LowLow | SubBandType::LowHigh => ZERO_CTX_LL_LH_LOOKUP[neighbors as usize],
        SubBandType::HighLow => ZERO_CTX_HL_LOOKUP[neighbors as usize],
        SubBandType::HighHigh => ZERO_CTX_HH_LOOKUP[neighbors as usize],
    }
}

/// Return the context label for magnitude refinement coding (Table D.4).
fn context_label_magnitude_refinement_coding(pos: Position, ctx: &CodeBlockDecodeContext) -> u8 {
    // If magnitude refined, then 16.
    let m1 = ctx.magnitude_refinement(pos) * 16;
    // Else: If at least one neighbor is significant then 15, else 14.
    let m2 = 14 + ctx.neighborhood_significance_states(pos).min(1);

    u8::max(m1, m2)
}

#[derive(Default, Copy, Clone, Debug)]
struct Position {
    index_x: u32,
    index_y: u32,
}

impl Position {
    fn new(x: u32, y: u32) -> Position {
        Self {
            index_x: x + 1,
            index_y: y + 1,
        }
    }

    fn new_index(x: u32, y: u32) -> Position {
        Self {
            index_x: x,
            index_y: y,
        }
    }

    fn left(&self) -> Position {
        Self::new_index(self.index_x - 1, self.index_y)
    }

    fn right(&self) -> Position {
        Self::new_index(self.index_x + 1, self.index_y)
    }

    fn top(&self) -> Position {
        Self::new_index(self.index_x, self.index_y - 1)
    }

    fn bottom(&self) -> Position {
        Self::new_index(self.index_x, self.index_y + 1)
    }

    fn top_left(&self) -> Position {
        Self::new_index(self.index_x - 1, self.index_y - 1)
    }

    fn top_right(&self) -> Position {
        Self::new_index(self.index_x + 1, self.index_y - 1)
    }

    fn bottom_left(&self) -> Position {
        Self::new_index(self.index_x - 1, self.index_y + 1)
    }

    fn bottom_right(&self) -> Position {
        Self::new_index(self.index_x + 1, self.index_y + 1)
    }

    fn real_y(&self) -> u32 {
        self.index_y - 1
    }

    fn index(&self, padded_width: u32) -> usize {
        self.index_x as usize + self.index_y as usize * padded_width as usize
    }
}

// We use a trait so that we can mock the arithmetic decoder for tests.
trait BitDecoder {
    const IS_BYPASS: bool;

    fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> Option<u32>;
}

impl BitDecoder for ArithmeticDecoder<'_> {
    const IS_BYPASS: bool = false;

    #[inline(always)]
    fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> Option<u32> {
        Some(Self::read_bit(self, context))
    }
}

struct BypassDecoder<'a>(BitReader<'a>, bool);

impl<'a> BypassDecoder<'a> {
    fn new(data: &'a [u8], strict: bool) -> Self {
        Self(BitReader::new(data), strict)
    }
}

impl BitDecoder for BypassDecoder<'_> {
    const IS_BYPASS: bool = true;

    fn read_bit(&mut self, _: &mut ArithmeticDecoderContext) -> Option<u32> {
        self.0.read_bits_with_stuffing(1).or({
            if !self.1 {
                // Just pad with ones. Not sure if zeroes would be better here,
                // but since the arithmetic decoder is also padded with 0xFF
                // maybe 1 is the better choice?
                Some(1)
            } else {
                None
            }
        })
    }
}
