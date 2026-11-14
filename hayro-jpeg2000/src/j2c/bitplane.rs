use alloc::vec;
use alloc::vec::Vec;

use super::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use super::build::{CodeBlock, SubBandType};
use super::codestream::CodeBlockStyle;
use super::decode::{DecompositionStorage, TileDecodeContext};
use crate::error::{DecodingError, Result, bail};
use crate::reader::BitReader;

pub(crate) fn decode(
    code_block: &CodeBlock,
    sub_band_type: SubBandType,
    total_bitplanes: u8,
    style: &CodeBlockStyle,
    tile_ctx: &mut TileDecodeContext,
    storage: &DecompositionStorage<'_>,
    strict: bool,
) -> Result<()> {
    tile_ctx.bit_plane_decode_context.reset(
        code_block,
        sub_band_type,
        style,
        total_bitplanes,
        strict,
    )?;
    tile_ctx.bit_plane_decode_buffers.reset();

    decode_inner(
        code_block,
        storage,
        &mut tile_ctx.bit_plane_decode_context,
        &mut tile_ctx.bit_plane_decode_buffers,
    )
    .ok_or(DecodingError::CodeBlockDecodeFailure)?;

    Ok(())
}

fn decode_inner(
    code_block: &CodeBlock,
    storage: &DecompositionStorage<'_>,
    ctx: &mut BitPlaneDecodeContext,
    bp_buffers: &mut BitPlaneDecodeBuffers,
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
        let mut decoder = ArithmeticDecoder::new(&bp_buffers.combined_layers);
        let end = code_block
            .number_of_coding_passes
            .min(ctx.max_coding_passes);

        if ctx.can_use_fast_path() {
            fast_path::handle_coding_passes(0, end, ctx, &mut decoder)?;
        } else {
            handle_coding_passes(0, end, ctx, &mut decoder)?;
        }
    } else {
        for segment in 0..bp_buffers.segment_coding_passes.len() - 1 {
            let start_coding_pass = bp_buffers.segment_coding_passes[segment];
            let end_coding_pass =
                bp_buffers.segment_coding_passes[segment + 1].min(ctx.max_coding_passes);

            let data = &bp_buffers.combined_layers
                [bp_buffers.segment_ranges[segment]..bp_buffers.segment_ranges[segment + 1]];

            let use_arithmetic = if ctx.style.selective_arithmetic_coding_bypass {
                if start_coding_pass <= 9 {
                    true
                } else {
                    start_coding_pass.is_multiple_of(3)
                }
            } else {
                true
            };

            if use_arithmetic {
                let mut decoder = ArithmeticDecoder::new(data);
                handle_coding_passes(start_coding_pass, end_coding_pass, ctx, &mut decoder)?;
            } else {
                let mut decoder = BypassDecoder::new(data, ctx.strict);
                handle_coding_passes(start_coding_pass, end_coding_pass, ctx, &mut decoder)?;
            }
        }
    }

    Some(())
}

fn handle_coding_passes(
    start: u8,
    end: u8,
    ctx: &mut BitPlaneDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    let reset_context_probabilities = ctx.style.reset_context_probabilities;

    for coding_pass in start..end {
        let current_bitplane = coding_pass.div_ceil(3);
        ctx.current_bit_position = ctx.bitplanes - 1 - current_bitplane;

        match coding_pass % 3 {
            0 => {
                cleanup_pass(ctx, decoder)?;

                if ctx.style.segmentation_symbols {
                    let b0 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b1 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b2 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                    let b3 = decoder.read_bit(ctx.arithmetic_decoder_context(18))?;

                    if (b0 != 1 || b1 != 0 || b2 != 1 || b3 != 0) && ctx.strict {
                        return None;
                    }
                }

                ctx.reset_for_next_bitplane();
            }
            1 => {
                significance_propagation_pass(ctx, decoder)?;
            }
            2 => {
                magnitude_refinement_pass(ctx, decoder)?;
            }
            _ => unreachable!(),
        }

        if reset_context_probabilities {
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
const DECODED_BIT_POSITION_MASK: u8 = 0x1f;
const CLEANUP_SKIP_MASK: u8 = (1 << SIGNIFICANCE_SHIFT) | (1 << HAS_ZERO_CODING_SHIFT);
const REFINEMENT_PASS_MASK: u8 = (1 << SIGNIFICANCE_SHIFT) | (1 << HAS_ZERO_CODING_SHIFT);

#[derive(Default, Copy, Clone)]
pub(crate) struct CoefficientState(u8);

impl CoefficientState {
    #[inline(always)]
    fn set_significant(&mut self) {
        self.0 |= 1_u8 << SIGNIFICANCE_SHIFT;
    }

    #[inline(always)]
    fn set_zero_coded(&mut self, value: u8) {
        debug_assert!(value < 2);

        self.0 &= !(1_u8 << HAS_ZERO_CODING_SHIFT);
        self.0 |= value << HAS_ZERO_CODING_SHIFT;
    }

    #[inline(always)]
    fn set_magnitude_refined(&mut self) {
        self.0 |= 1_u8 << HAS_MAGNITUDE_REFINEMENT_SHIFT;
    }

    #[inline(always)]
    fn set_decoded_bit_position(&mut self, position: u8) {
        debug_assert!(position <= DECODED_BIT_POSITION_MASK);

        self.0 = (self.0 & !DECODED_BIT_POSITION_MASK) | position;
    }

    #[inline(always)]
    fn decoded_bit_position(&self) -> u8 {
        self.0 & DECODED_BIT_POSITION_MASK
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
    fn should_decode_cleanup(&self) -> bool {
        self.0 & CLEANUP_SKIP_MASK == 0
    }

    #[inline(always)]
    fn should_decode_refinement(&self) -> bool {
        self.0 & REFINEMENT_PASS_MASK == 1 << SIGNIFICANCE_SHIFT
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Coefficient(u32);

impl Coefficient {
    pub(crate) fn reconstructed(
        &self,
        state: &CoefficientState,
        irreversible: bool,
        roi_shift: u8,
    ) -> f32 {
        let mut magnitude = self.0 & !0x80000000;
        let mut bit_position = state.decoded_bit_position();

        let roi_magnitude = magnitude.checked_shr(roi_shift as u32).unwrap_or(0);
        if roi_magnitude != 0 {
            magnitude = roi_magnitude;
            bit_position = bit_position.saturating_sub(roi_shift);
        }

        let mut magnitude = magnitude as i32;
        magnitude *= 1 - 2 * (self.sign() as i32);

        if magnitude != 0 && (irreversible || bit_position != 0) {
            let offset = 0.5 * (1_u32 << bit_position) as f32;
            return magnitude as f32 + if magnitude > 0 { offset } else { -offset };
        }

        magnitude as f32
    }

    fn set_sign(&mut self, sign: u8) {
        self.0 |= (sign as u32) << 31;
    }

    fn sign(&self) -> u32 {
        (self.0 >> 31) & 1
    }

    fn push_bit_at(&mut self, bit: u32, position: u8) {
        self.0 |= bit << position;
    }
}

const COEFFICIENTS_PADDING: u32 = 1;

// Direct neighbors occupy the even bits so they can be masked in one step.
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

        // Fenceposts: segment `i` spans entries `i..i + 1`.
        self.segment_ranges.push(0);
        self.segment_coding_passes.push(0);
    }
}

pub(crate) struct BitPlaneDecodeContext {
    coefficient_states: Vec<CoefficientState>,
    stripe_flags: Vec<u32>,
    neighbor_significances: Vec<NeighborSignificances>,
    coefficients: Vec<Coefficient>,
    width: u32,
    padded_width: u32,
    height: u32,
    style: CodeBlockStyle,
    bitplanes: u8,
    strict: bool,
    max_coding_passes: u8,
    sub_band_type: SubBandType,
    contexts: [ArithmeticDecoderContext; 19],
    current_bit_position: u8,
}

impl Default for BitPlaneDecodeContext {
    fn default() -> Self {
        Self {
            coefficient_states: vec![],
            stripe_flags: vec![],
            coefficients: vec![],
            neighbor_significances: vec![],
            width: 0,
            padded_width: COEFFICIENTS_PADDING * 2,
            height: 0,
            style: CodeBlockStyle::default(),
            bitplanes: 0,
            max_coding_passes: 0,
            strict: false,
            sub_band_type: SubBandType::LowLow,
            contexts: [ArithmeticDecoderContext::default(); 19],
            current_bit_position: 0,
        }
    }
}

impl BitPlaneDecodeContext {
    fn can_use_fast_path(&self) -> bool {
        self.height.is_multiple_of(4) && !self.style.vertically_causal_context
    }

    pub(crate) fn reset(
        &mut self,
        code_block: &CodeBlock,
        sub_band_type: SubBandType,
        code_block_style: &CodeBlockStyle,
        total_bitplanes: u8,
        strict: bool,
    ) -> Result<()> {
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

        self.stripe_flags.clear();
        self.stripe_flags
            .resize((height.div_ceil(4) + 2) as usize * padded_width as usize, 0);

        self.width = width;
        self.padded_width = padded_width;
        self.height = height;
        self.sub_band_type = sub_band_type;
        self.style = *code_block_style;
        self.reset_contexts();

        // Lenient on files whose missing bitplanes exceed the total (issue 399).
        self.bitplanes = if strict {
            total_bitplanes
                .checked_sub(code_block.missing_bit_planes)
                .ok_or(DecodingError::InvalidBitplaneCount)?
        } else {
            total_bitplanes.saturating_sub(code_block.missing_bit_planes)
        };

        self.max_coding_passes = if self.bitplanes == 0 {
            0
        } else {
            1 + 3 * (self.bitplanes - 1)
        };

        if self.max_coding_passes < code_block.number_of_coding_passes && strict {
            bail!(DecodingError::TooManyCodingPasses);
        }

        Ok(())
    }

    pub(crate) fn coefficient_rows(
        &self,
    ) -> impl Iterator<Item = (&[Coefficient], &[CoefficientState])> {
        self.coefficients
            .chunks_exact(self.padded_width as usize)
            .zip(
                self.coefficient_states
                    .chunks_exact(self.padded_width as usize),
            )
            .map(|(coefficients, states)| {
                (
                    &coefficients[COEFFICIENTS_PADDING as usize..][..self.width as usize],
                    &states[COEFFICIENTS_PADDING as usize..][..self.width as usize],
                )
            })
            .skip(COEFFICIENTS_PADDING as usize)
            .take(self.height as usize)
    }

    fn set_sign(&mut self, pos: Position, sign: u8) {
        self.coefficients[pos.index(self.padded_width)].set_sign(sign);
    }

    fn arithmetic_decoder_context(&mut self, ctx_label: u8) -> &mut ArithmeticDecoderContext {
        &mut self.contexts[ctx_label as usize]
    }

    fn reset_contexts(&mut self) {
        for context in &mut self.contexts {
            context.reset();
        }

        self.contexts[0].reset_with_index(4);
        self.contexts[17].reset_with_index(3);
        self.contexts[18].reset_with_index(46);
    }

    fn reset_for_next_bitplane(&mut self) {
        for el in &mut self.coefficient_states {
            el.set_zero_coded(0);
        }
    }

    #[inline(always)]
    fn is_significant(&self, position: Position) -> bool {
        self.coefficient_states[position.index(self.padded_width)].is_significant()
    }

    #[inline(always)]
    fn set_significant(&mut self, position: Position) {
        let idx = position.index(self.padded_width);
        debug_assert!(!self.coefficient_states[idx].is_significant());

        self.coefficient_states[idx].set_significant();
        self.coefficient_states[idx].set_decoded_bit_position(self.current_bit_position);

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

    #[inline(always)]
    fn set_zero_coded(&mut self, position: Position) {
        self.coefficient_states[position.index(self.padded_width)].set_zero_coded(1);
    }

    #[inline(always)]
    fn set_magnitude_refined(&mut self, position: Position) {
        let state = &mut self.coefficient_states[position.index(self.padded_width)];
        state.set_magnitude_refined();
        state.set_decoded_bit_position(self.current_bit_position);
    }

    #[inline(always)]
    fn magnitude_refinement(&self, position: Position) -> u8 {
        self.coefficient_states[position.index(self.padded_width)].magnitude_refinement()
    }

    #[inline(always)]
    fn should_decode_cleanup(&self, position: Position) -> bool {
        self.coefficient_states[position.index(self.padded_width)].should_decode_cleanup()
    }

    #[inline(always)]
    fn should_decode_refinement(&self, position: Position) -> bool {
        self.coefficient_states[position.index(self.padded_width)].should_decode_refinement()
    }

    #[inline(always)]
    fn push_magnitude_bit(&mut self, position: Position, bit: u32) {
        let idx = position.index(self.padded_width);
        self.coefficients[idx].push_bit_at(bit, self.current_bit_position);
    }

    #[inline(always)]
    fn sign(&self, position: Position) -> u8 {
        self.coefficients[position.index(self.padded_width)].sign() as u8
    }

    #[inline(always)]
    fn neighbor_in_next_stripe(&self, pos: Position) -> bool {
        let neighbor = pos.bottom();
        neighbor.real_y() < self.height && (neighbor.real_y() >> 2) > (pos.real_y() >> 2)
    }

    #[inline(always)]
    fn neighborhood_significance_states(&self, pos: Position) -> u8 {
        let neighbors = &self.neighbor_significances[pos.index(self.padded_width)];

        if self.style.vertically_causal_context && self.neighbor_in_next_stripe(pos) {
            neighbors.all_without_bottom()
        } else {
            neighbors.all()
        }
    }
}

fn cleanup_pass(ctx: &mut BitPlaneDecodeContext, decoder: &mut impl BitDecoder) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            if ctx.should_decode_cleanup(*cur_pos) {
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
                    let bit = decoder.read_bit(ctx.arithmetic_decoder_context(17))?;

                    if bit == 0 {
                        for _ in 0..3 {
                            cur_pos.index_y += 1;
                        }

                        return Some(());
                    } else {
                        let mut num_zeroes =
                            decoder.read_bit(ctx.arithmetic_decoder_context(18))?;
                        num_zeroes = (num_zeroes << 1)
                            | decoder.read_bit(ctx.arithmetic_decoder_context(18))?;

                        for _ in 0..num_zeroes {
                            cur_pos.index_y += 1;
                        }

                        1
                    }
                } else {
                    let ctx_label = context_label_zero_coding(*cur_pos, ctx);
                    decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?
                };

                if bit == 1 {
                    ctx.push_magnitude_bit(*cur_pos, bit);
                    decode_sign_bit(*cur_pos, ctx, decoder);
                    ctx.set_significant(*cur_pos);
                }
            }

            Some(())
        },
    )
}

fn significance_propagation_pass(
    ctx: &mut BitPlaneDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            if !ctx.is_significant(*cur_pos) && ctx.neighborhood_significance_states(*cur_pos) != 0
            {
                let ctx_label = context_label_zero_coding(*cur_pos, ctx);
                let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?;
                ctx.set_zero_coded(*cur_pos);

                if bit == 1 {
                    ctx.push_magnitude_bit(*cur_pos, bit);
                    decode_sign_bit(*cur_pos, ctx, decoder)?;
                    ctx.set_significant(*cur_pos);
                }
            }

            Some(())
        },
    )
}

fn magnitude_refinement_pass(
    ctx: &mut BitPlaneDecodeContext,
    decoder: &mut impl BitDecoder,
) -> Option<()> {
    for_each_position(
        ctx.width,
        ctx.height,
        #[inline(always)]
        |cur_pos| {
            if ctx.should_decode_refinement(*cur_pos) {
                let ctx_label = context_label_magnitude_refinement_coding(*cur_pos, ctx);
                let bit = decoder.read_bit(ctx.arithmetic_decoder_context(ctx_label))?;
                if bit == 1 {
                    ctx.push_magnitude_bit(*cur_pos, bit);
                }
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

#[rustfmt::skip]
const SIGN_CONTEXT_LOOKUP: [(u8, u8); 256] = [
    (9,0), (10,0), (10,1), (0,0), (12,0), (13,0), (11,0), (0,0), (12,1), (11,1), 
    (13,1), (0,0), (0,0), (0,0), (0,0), (0,0), (12,0), (13,0), (11,0), (0,0), 
    (12,0), (13,0), (11,0), (0,0), (9,0), (10,0), (10,1), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (12,1), (11,1), (13,1), (0,0), (9,0), (10,0), (10,1), (0,0), 
    (12,1), (11,1), (13,1), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
    (0,0), (0,0), (0,0), (10,0), (10,0), (9,0), (0,0), (13,0), (13,0), (12,0), 
    (0,0), (11,1), (11,1), (12,1), (0,0), (0,0), (0,0), (0,0), (0,0), (13,0), 
    (13,0), (12,0), (0,0), (13,0), (13,0), (12,0), (0,0), (10,0), (10,0), (9,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (11,1), (11,1), (12,1), (0,0), (10,0), 
    (10,0), (9,0), (0,0), (11,1), (11,1), (12,1), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (10,1), (9,0), (10,1), (0,0), 
    (11,0), (12,0), (11,0), (0,0), (13,1), (12,1), (13,1), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (11,0), (12,0), (11,0), (0,0), (11,0), (12,0), (11,0), (0,0), 
    (10,1), (9,0), (10,1), (0,0), (0,0), (0,0), (0,0), (0,0), (13,1), (12,1), 
    (13,1), (0,0), (10,1), (9,0), (10,1), (0,0), (13,1), (12,1), (13,1), (0,0),
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), 
    (0,0), (0,0), (0,0), (0,0), (0,0), (0,0), (0,0),
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

#[inline(always)]
fn decode_sign_bit<T: BitDecoder>(
    pos: Position,
    ctx: &mut BitPlaneDecodeContext,
    decoder: &mut T,
) -> Option<()> {
    #[inline(always)]
    fn context_label_sign_coding(pos: Position, ctx: &BitPlaneDecodeContext) -> (u8, u8) {
        let significances = ctx.neighborhood_significance_states(pos) & 0b0101_0101;

        let left_sign = ctx.sign(pos.left());
        let right_sign = ctx.sign(pos.right());
        let top_sign = ctx.sign(pos.top());
        let bottom_sign = if ctx.style.vertically_causal_context && ctx.neighbor_in_next_stripe(pos)
        {
            0
        } else {
            ctx.sign(pos.bottom())
        };

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

#[inline(always)]
fn context_label_zero_coding(pos: Position, ctx: &BitPlaneDecodeContext) -> u8 {
    let neighbors = ctx.neighborhood_significance_states(pos);

    match ctx.sub_band_type {
        SubBandType::LowLow | SubBandType::LowHigh => ZERO_CTX_LL_LH_LOOKUP[neighbors as usize],
        SubBandType::HighLow => ZERO_CTX_HL_LOOKUP[neighbors as usize],
        SubBandType::HighHigh => ZERO_CTX_HH_LOOKUP[neighbors as usize],
    }
}

fn context_label_magnitude_refinement_coding(pos: Position, ctx: &BitPlaneDecodeContext) -> u8 {
    let m1 = ctx.magnitude_refinement(pos) * 16;
    let m2 = 14 + ctx.neighborhood_significance_states(pos).min(1);

    u8::max(m1, m2)
}

#[derive(Default, Copy, Clone, Debug)]
struct Position {
    // Offset by `COEFFICIENTS_PADDING`.
    index_x: u32,
    index_y: u32,
}

impl Position {
    fn new(x: u32, y: u32) -> Self {
        Self {
            index_x: x + COEFFICIENTS_PADDING,
            index_y: y + COEFFICIENTS_PADDING,
        }
    }

    fn new_index(x: u32, y: u32) -> Self {
        Self {
            index_x: x,
            index_y: y,
        }
    }

    fn left(&self) -> Self {
        Self::new_index(self.index_x - 1, self.index_y)
    }

    fn right(&self) -> Self {
        Self::new_index(self.index_x + 1, self.index_y)
    }

    fn top(&self) -> Self {
        Self::new_index(self.index_x, self.index_y - 1)
    }

    fn bottom(&self) -> Self {
        Self::new_index(self.index_x, self.index_y + 1)
    }

    fn top_left(&self) -> Self {
        Self::new_index(self.index_x - 1, self.index_y - 1)
    }

    fn top_right(&self) -> Self {
        Self::new_index(self.index_x + 1, self.index_y - 1)
    }

    fn bottom_left(&self) -> Self {
        Self::new_index(self.index_x - 1, self.index_y + 1)
    }

    fn bottom_right(&self) -> Self {
        Self::new_index(self.index_x + 1, self.index_y + 1)
    }

    fn real_y(&self) -> u32 {
        self.index_y - 1
    }

    fn index(&self, padded_width: u32) -> usize {
        self.index_x as usize + self.index_y as usize * padded_width as usize
    }
}

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
                // Pad with ones, matching the arithmetic decoder's 0xFF padding.
                Some(1)
            } else {
                None
            }
        })
    }
}

mod fast_path {
    use super::{
        ArithmeticDecoder, BitPlaneDecodeContext, COEFFICIENTS_PADDING, SIGN_CONTEXT_LOOKUP,
        SubBandType, ZERO_CTX_HH_LOOKUP, ZERO_CTX_HL_LOOKUP, ZERO_CTX_LL_LH_LOOKUP,
    };

    // Note: Credit where credit is due, this fast path borrows some core ideas
    // from the OpenJPEG implementation (e.g. the idea to bitpack state into u32),
    // but has been adapted to more naturally work with our implementation.

    pub(super) fn handle_coding_passes(
        start: u8,
        end: u8,
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
    ) -> Option<()> {
        let reset_context_probabilities = ctx.style.reset_context_probabilities;

        for coding_pass in start..end {
            let current_bitplane = coding_pass.div_ceil(3);
            ctx.current_bit_position = ctx.bitplanes - 1 - current_bitplane;

            match coding_pass % 3 {
                0 => {
                    cleanup_pass(ctx, decoder);

                    if ctx.style.segmentation_symbols {
                        let b0 = decoder.read_bit(&mut ctx.contexts[18]);
                        let b1 = decoder.read_bit(&mut ctx.contexts[18]);
                        let b2 = decoder.read_bit(&mut ctx.contexts[18]);
                        let b3 = decoder.read_bit(&mut ctx.contexts[18]);

                        if (b0 != 1 || b1 != 0 || b2 != 1 || b3 != 0) && ctx.strict {
                            return None;
                        }
                    }

                    reset_for_next_bitplane(ctx);
                }
                1 => significance_propagation_pass(ctx, decoder),
                2 => magnitude_refinement_pass(ctx, decoder),
                _ => unreachable!(),
            }

            if reset_context_probabilities {
                ctx.reset_contexts();
            }
        }

        Some(())
    }

    fn cleanup_pass(ctx: &mut BitPlaneDecodeContext, decoder: &mut ArithmeticDecoder<'_>) {
        let zero_contexts = zero_context_lookup(ctx.sub_band_type);

        for_each_stripe_column(
            ctx,
            #[inline(always)]
            |ctx, column| {
                let mut flags = column.flags;
                let use_rl = flags == 0;

                macro_rules! cleanup {
                    ($coefficient_in_stripe:expr) => {
                        cleanup_coefficient(
                            ctx,
                            decoder,
                            zero_contexts,
                            column,
                            &mut flags,
                            $coefficient_in_stripe,
                            true,
                            false,
                        )
                    };
                }

                if use_rl {
                    let bit = decoder.read_bit(&mut ctx.contexts[17]);

                    if bit == 1 {
                        let mut num_zeroes = decoder.read_bit(&mut ctx.contexts[18]);
                        num_zeroes = (num_zeroes << 1) | decoder.read_bit(&mut ctx.contexts[18]);
                        cleanup_from_run_length(
                            ctx,
                            decoder,
                            zero_contexts,
                            column,
                            &mut flags,
                            num_zeroes,
                        );
                    }
                } else {
                    cleanup!(0);
                    cleanup!(1);
                    cleanup!(2);
                    cleanup!(3);
                }

                flags & !FLAG_ZERO_CODED_ALL
            },
        );
    }

    #[inline(always)]
    fn cleanup_from_run_length(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
        zero_contexts: &[u8; 512],
        column: StripeColumn,
        flags: &mut u32,
        num_zeroes: u32,
    ) {
        macro_rules! decode {
            ($coefficient_in_stripe:expr) => {
                cleanup_coefficient(
                    ctx,
                    decoder,
                    zero_contexts,
                    column,
                    flags,
                    $coefficient_in_stripe,
                    false,
                    false,
                )
            };
        }

        macro_rules! significant {
            ($coefficient_in_stripe:expr) => {
                cleanup_coefficient(
                    ctx,
                    decoder,
                    zero_contexts,
                    column,
                    flags,
                    $coefficient_in_stripe,
                    false,
                    true,
                )
            };
        }

        match num_zeroes {
            0 => {
                significant!(0);
                decode!(1);
                decode!(2);
                decode!(3);
            }
            1 => {
                significant!(1);
                decode!(2);
                decode!(3);
            }
            2 => {
                significant!(2);
                decode!(3);
            }
            _ => significant!(3),
        }
    }

    #[inline(always)]
    fn cleanup_coefficient(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
        zero_contexts: &[u8; 512],
        column: StripeColumn,
        flags: &mut u32,
        coefficient_in_stripe: u32,
        check_flags: bool,
        already_known_significant: bool,
    ) {
        debug_assert!(coefficient_in_stripe < 4);
        let shift = coefficient_in_stripe * FLAGS_PER_COEFFICIENT;

        if !check_flags || (*flags & ((FLAG_SIGNIFICANT_THIS | FLAG_ZERO_CODED_THIS) << shift)) == 0
        {
            if !already_known_significant {
                let ctx_label =
                    zero_contexts[((*flags >> shift) & FLAG_SIGNIFICANT_NEIGHBORS) as usize];
                if decoder.read_bit(&mut ctx.contexts[ctx_label as usize]) == 0 {
                    return;
                }
            }

            let coefficient_idx =
                column.coefficient_idx + coefficient_in_stripe as usize * column.stride;
            push_magnitude_bit(ctx, coefficient_idx, 1);
            let sign = decode_sign_bit(ctx, decoder, column, *flags, coefficient_in_stripe);
            set_sign(ctx, coefficient_idx, sign as u8);
            set_significant(ctx, column, flags, coefficient_in_stripe, sign);
        }
    }

    fn significance_propagation_pass(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
    ) {
        let zero_contexts = zero_context_lookup(ctx.sub_band_type);

        for_each_stripe_column(
            ctx,
            #[inline(always)]
            |ctx, column| {
                let mut flags = column.flags;

                macro_rules! propagate {
                    ($coefficient_in_stripe:expr) => {
                        significance_propagation_coefficient(
                            ctx,
                            decoder,
                            zero_contexts,
                            column,
                            &mut flags,
                            $coefficient_in_stripe,
                        )
                    };
                }

                if flags != 0 {
                    propagate!(0);
                    propagate!(1);
                    propagate!(2);
                    propagate!(3);
                }

                flags
            },
        );
    }

    #[inline(always)]
    fn significance_propagation_coefficient(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
        zero_contexts: &[u8; 512],
        column: StripeColumn,
        flags: &mut u32,
        coefficient_in_stripe: u32,
    ) {
        debug_assert!(coefficient_in_stripe < 4);
        let shift = coefficient_in_stripe * FLAGS_PER_COEFFICIENT;
        let shifted_flags = *flags >> shift;
        let should_propagate = shifted_flags & (FLAG_SIGNIFICANT_THIS | FLAG_ZERO_CODED_THIS) == 0
            && shifted_flags & FLAG_SIGNIFICANT_NEIGHBORS != 0;

        if should_propagate {
            let ctx_label = zero_contexts[(shifted_flags & FLAG_SIGNIFICANT_NEIGHBORS) as usize];
            let bit = decoder.read_bit(&mut ctx.contexts[ctx_label as usize]);
            *flags |= FLAG_ZERO_CODED_THIS << shift;

            if bit == 1 {
                let coefficient_idx =
                    column.coefficient_idx + coefficient_in_stripe as usize * column.stride;
                push_magnitude_bit(ctx, coefficient_idx, bit);

                let sign = decode_sign_bit(ctx, decoder, column, *flags, coefficient_in_stripe);
                set_sign(ctx, coefficient_idx, sign as u8);
                set_significant(ctx, column, flags, coefficient_in_stripe, sign);
            }
        }
    }

    fn magnitude_refinement_pass(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
    ) {
        for_each_stripe_column(
            ctx,
            #[inline(always)]
            |ctx, column| {
                let mut flags = column.flags;

                macro_rules! refine {
                    ($coefficient_in_stripe:expr) => {
                        magnitude_refinement_coefficient(
                            ctx,
                            decoder,
                            column,
                            &mut flags,
                            $coefficient_in_stripe,
                        )
                    };
                }

                if flags != 0 {
                    refine!(0);
                    refine!(1);
                    refine!(2);
                    refine!(3);
                }

                flags
            },
        );
    }

    #[inline(always)]
    fn magnitude_refinement_coefficient(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
        column: StripeColumn,
        flags: &mut u32,
        coefficient_in_stripe: u32,
    ) {
        let shift = coefficient_in_stripe * FLAGS_PER_COEFFICIENT;
        let shifted_flags = *flags >> shift;
        let should_decode_refinement =
            shifted_flags & (FLAG_SIGNIFICANT_THIS | FLAG_ZERO_CODED_THIS) == FLAG_SIGNIFICANT_THIS;

        if should_decode_refinement {
            let ctx_label = magnitude_refinement_context(shifted_flags);
            let bit = decoder.read_bit(&mut ctx.contexts[ctx_label as usize]);
            let coefficient_idx =
                column.coefficient_idx + coefficient_in_stripe as usize * column.stride;
            if bit == 1 {
                push_magnitude_bit(ctx, coefficient_idx, bit);
            }
            ctx.coefficient_states[coefficient_idx]
                .set_decoded_bit_position(ctx.current_bit_position);
            *flags |= FLAG_MAGNITUDE_REFINED_THIS << shift;
        }
    }

    #[inline(always)]
    fn decode_sign_bit(
        ctx: &mut BitPlaneDecodeContext,
        decoder: &mut ArithmeticDecoder<'_>,
        column: StripeColumn,
        flags: u32,
        coefficient_in_stripe: u32,
    ) -> u32 {
        let shift = coefficient_in_stripe * FLAGS_PER_COEFFICIENT;
        let shifted_flags = flags >> shift;
        let left_flags = ctx.stripe_flags[column.flag_idx - 1];
        let right_flags = ctx.stripe_flags[column.flag_idx + 1];

        let top_sign = if coefficient_in_stripe == 0 {
            (flags >> FLAG_SIGN_TOP_STRIPE_SHIFT) & 1
        } else {
            (flags >> (FLAG_SIGN_THIS_SHIFT + (coefficient_in_stripe - 1) * 3)) & 1
        };
        let bottom_sign = (flags
            >> (FLAG_SIGN_BOTTOM_THIS_SHIFT + coefficient_in_stripe * FLAGS_PER_COEFFICIENT))
            & 1;
        let left_sign = (left_flags >> (FLAG_SIGN_THIS_SHIFT + shift)) & 1;
        let right_sign = (right_flags >> (FLAG_SIGN_THIS_SHIFT + shift)) & 1;

        let lookup = left_sign
            | (((shifted_flags & FLAG_SIGNIFICANT_TOP) != 0) as u32) << 1
            | (right_sign << 2)
            | (((shifted_flags & FLAG_SIGNIFICANT_LEFT) != 0) as u32) << 3
            | (top_sign << 4)
            | (((shifted_flags & FLAG_SIGNIFICANT_RIGHT) != 0) as u32) << 5
            | (bottom_sign << 6)
            | (((shifted_flags & FLAG_SIGNIFICANT_BOTTOM) != 0) as u32) << 7;

        let (ctx_label, xor_bit) = SIGN_CONTEXT_STRIPE_LOOKUP[lookup as usize];
        decoder.read_bit(&mut ctx.contexts[ctx_label as usize]) ^ xor_bit as u32
    }

    #[inline(always)]
    fn zero_context_lookup(sub_band_type: SubBandType) -> &'static [u8; 512] {
        match sub_band_type {
            SubBandType::LowLow | SubBandType::LowHigh => &ZERO_CTX_LL_LH_STRIPE_LOOKUP,
            SubBandType::HighLow => &ZERO_CTX_HL_STRIPE_LOOKUP,
            SubBandType::HighHigh => &ZERO_CTX_HH_STRIPE_LOOKUP,
        }
    }

    #[inline(always)]
    fn magnitude_refinement_context(flags: u32) -> u8 {
        if flags & FLAG_MAGNITUDE_REFINED_THIS != 0 {
            16
        } else {
            14 + ((flags & FLAG_SIGNIFICANT_NEIGHBORS != 0) as u8)
        }
    }

    #[inline(always)]
    fn reset_for_next_bitplane(ctx: &mut BitPlaneDecodeContext) {
        for flags in &mut ctx.stripe_flags {
            *flags &= !FLAG_ZERO_CODED_ALL;
        }
    }

    #[derive(Copy, Clone)]
    struct StripeColumn {
        flag_idx: usize,
        coefficient_idx: usize,
        flags: u32,
        stride: usize,
    }

    #[inline(always)]
    fn for_each_stripe_column(
        ctx: &mut BitPlaneDecodeContext,
        mut action: impl FnMut(&mut BitPlaneDecodeContext, StripeColumn) -> u32,
    ) {
        let width = ctx.width as usize;
        let stride = ctx.padded_width as usize;
        let stripe_count = (ctx.height / 4) as usize;
        let padding = COEFFICIENTS_PADDING as usize;
        let mut flag_base = padding * stride + padding;
        let mut coefficient_base = padding * stride + padding;

        for _ in 0..stripe_count {
            for x in 0..width {
                let flag_idx = flag_base + x;
                let column = StripeColumn {
                    flag_idx,
                    coefficient_idx: coefficient_base + x,
                    flags: ctx.stripe_flags[flag_idx],
                    stride,
                };
                let flags = action(ctx, column);

                ctx.stripe_flags[flag_idx] = flags;
            }

            flag_base += stride;
            coefficient_base += 4 * stride;
        }
    }

    #[inline(always)]
    fn push_magnitude_bit(ctx: &mut BitPlaneDecodeContext, idx: usize, bit: u32) {
        ctx.coefficients[idx].push_bit_at(bit, ctx.current_bit_position);
    }

    #[inline(always)]
    fn set_sign(ctx: &mut BitPlaneDecodeContext, idx: usize, sign: u8) {
        ctx.coefficients[idx].set_sign(sign);
    }

    #[inline(always)]
    fn set_significant(
        ctx: &mut BitPlaneDecodeContext,
        column: StripeColumn,
        flags: &mut u32,
        coefficient_in_stripe: u32,
        sign: u32,
    ) {
        let shift = coefficient_in_stripe * FLAGS_PER_COEFFICIENT;
        let coefficient_idx =
            column.coefficient_idx + coefficient_in_stripe as usize * column.stride;
        ctx.coefficient_states[coefficient_idx].set_decoded_bit_position(ctx.current_bit_position);

        ctx.stripe_flags[column.flag_idx - 1] |= FLAG_SIGNIFICANT_RIGHT << shift;
        *flags |= ((sign << FLAG_SIGN_THIS_SHIFT) | FLAG_SIGNIFICANT_THIS) << shift;
        ctx.stripe_flags[column.flag_idx + 1] |= FLAG_SIGNIFICANT_LEFT << shift;

        if coefficient_in_stripe == 0 {
            let top = column.flag_idx - column.stride;
            ctx.stripe_flags[top] |= (sign << FLAG_SIGN_BOTTOM_STRIPE_SHIFT)
                | (FLAG_SIGNIFICANT_BOTTOM << LAST_COEFFICIENT_SHIFT);
            ctx.stripe_flags[top - 1] |= FLAG_SIGNIFICANT_BOTTOM_RIGHT << LAST_COEFFICIENT_SHIFT;
            ctx.stripe_flags[top + 1] |= FLAG_SIGNIFICANT_BOTTOM_LEFT << LAST_COEFFICIENT_SHIFT;
        }

        if coefficient_in_stripe == 3 {
            let bottom = column.flag_idx + column.stride;
            ctx.stripe_flags[bottom] |= (sign << FLAG_SIGN_TOP_STRIPE_SHIFT) | FLAG_SIGNIFICANT_TOP;
            ctx.stripe_flags[bottom - 1] |= FLAG_SIGNIFICANT_TOP_RIGHT;
            ctx.stripe_flags[bottom + 1] |= FLAG_SIGNIFICANT_TOP_LEFT;
        }
    }

    // Lane n's flags are the ones below shifted by 3n; bits 18/31: signs above/below the stripe.
    const FLAGS_PER_COEFFICIENT: u32 = 3;
    const LAST_COEFFICIENT_SHIFT: u32 = 3 * FLAGS_PER_COEFFICIENT;

    const FLAG_SIGNIFICANT_TOP_LEFT: u32 = 1 << 0;
    const FLAG_SIGNIFICANT_TOP: u32 = 1 << 1;
    const FLAG_SIGNIFICANT_TOP_RIGHT: u32 = 1 << 2;
    const FLAG_SIGNIFICANT_LEFT: u32 = 1 << 3;
    const FLAG_SIGNIFICANT_THIS: u32 = 1 << 4;
    const FLAG_SIGNIFICANT_RIGHT: u32 = 1 << 5;
    const FLAG_SIGNIFICANT_BOTTOM_LEFT: u32 = 1 << 6;
    const FLAG_SIGNIFICANT_BOTTOM: u32 = 1 << 7;
    const FLAG_SIGNIFICANT_BOTTOM_RIGHT: u32 = 1 << 8;
    const FLAG_SIGNIFICANT_NEIGHBORS: u32 = FLAG_SIGNIFICANT_TOP_LEFT
        | FLAG_SIGNIFICANT_TOP
        | FLAG_SIGNIFICANT_TOP_RIGHT
        | FLAG_SIGNIFICANT_LEFT
        | FLAG_SIGNIFICANT_RIGHT
        | FLAG_SIGNIFICANT_BOTTOM_LEFT
        | FLAG_SIGNIFICANT_BOTTOM
        | FLAG_SIGNIFICANT_BOTTOM_RIGHT;

    const FLAG_SIGN_TOP_STRIPE_SHIFT: u32 = 18;
    const FLAG_SIGN_THIS_SHIFT: u32 = 19;
    const FLAG_MAGNITUDE_REFINED_THIS: u32 = 1 << 20;
    const FLAG_ZERO_CODED_THIS: u32 = 1 << 21;
    const FLAG_SIGN_BOTTOM_THIS_SHIFT: u32 = 22;
    const FLAG_SIGN_BOTTOM_STRIPE_SHIFT: u32 = 31;
    const FLAG_ZERO_CODED_ALL: u32 = (1 << 21) | (1 << 24) | (1 << 27) | (1 << 30);

    const SIGN_CONTEXT_STRIPE_LOOKUP: [(u8, u8); 256] = build_sign_context_lookup();

    const fn build_sign_context_lookup() -> [(u8, u8); 256] {
        let mut out = [(0, 0); 256];
        let mut lookup = 0;

        while lookup < 256 {
            let left_sign = lookup & 1;
            let top_significance = (lookup >> 1) & 1;
            let right_sign = (lookup >> 2) & 1;
            let left_significance = (lookup >> 3) & 1;
            let top_sign = (lookup >> 4) & 1;
            let right_significance = (lookup >> 5) & 1;
            let bottom_sign = (lookup >> 6) & 1;
            let bottom_significance = (lookup >> 7) & 1;

            let significances = (top_significance << 6)
                | (left_significance << 4)
                | (right_significance << 2)
                | bottom_significance;
            let signs = (top_sign << 6) | (left_sign << 4) | (right_sign << 2) | bottom_sign;
            let merged_significances = ((significances & signs) << 1) | (significances & !signs);

            out[lookup] = SIGN_CONTEXT_LOOKUP[merged_significances];
            lookup += 1;
        }

        out
    }

    const ZERO_CTX_LL_LH_STRIPE_LOOKUP: [u8; 512] =
        build_zero_context_lookup(ZERO_CTX_LL_LH_LOOKUP);
    const ZERO_CTX_HL_STRIPE_LOOKUP: [u8; 512] = build_zero_context_lookup(ZERO_CTX_HL_LOOKUP);
    const ZERO_CTX_HH_STRIPE_LOOKUP: [u8; 512] = build_zero_context_lookup(ZERO_CTX_HH_LOOKUP);

    const fn build_zero_context_lookup(source: [u8; 256]) -> [u8; 512] {
        let mut out = [0; 512];
        let mut flags = 0;

        while flags < 512 {
            out[flags] = source[neighbor_byte(flags)];
            flags += 1;
        }

        out
    }

    const fn neighbor_byte(flags: usize) -> usize {
        ((((flags & FLAG_SIGNIFICANT_TOP_LEFT as usize) != 0) as usize) << 7)
            | ((((flags & FLAG_SIGNIFICANT_TOP as usize) != 0) as usize) << 6)
            | ((((flags & FLAG_SIGNIFICANT_TOP_RIGHT as usize) != 0) as usize) << 5)
            | ((((flags & FLAG_SIGNIFICANT_LEFT as usize) != 0) as usize) << 4)
            | ((((flags & FLAG_SIGNIFICANT_BOTTOM_LEFT as usize) != 0) as usize) << 3)
            | ((((flags & FLAG_SIGNIFICANT_RIGHT as usize) != 0) as usize) << 2)
            | ((((flags & FLAG_SIGNIFICANT_BOTTOM_RIGHT as usize) != 0) as usize) << 1)
            | (((flags & FLAG_SIGNIFICANT_BOTTOM as usize) != 0) as usize)
    }
}
