//! Decoding JPEG2000 code streams.
//!
//! This is the "core" module of the crate that orchestrates all
//! stages in such a way that a given codestream is decoded into its
//! component channels.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::bitplane::{BitPlaneDecodeBuffers, BitPlaneDecodeContext};
use super::build::{CodeBlock, Decomposition, Layer, Precinct, Segment, SubBand, SubBandType};
use super::codestream::{ComponentInfo, Header, QuantizationStyle, WaveletTransform};
use super::idwt::IDWTOutput;
use super::progression::{ProgressionData, progression_iterator};
use super::rect::IntRect;
use super::tag_tree::TagNode;
use super::tile::{ComponentTile, ResolutionTile, Tile};
use super::{ComponentData, bitplane, build, idwt, mct, segment, tile};
use crate::error::{DecodingError, Result, TileError, ValidationError, bail};
use crate::j2c::segment::MAX_BITPLANE_COUNT;
use crate::math::SimdBuffer;
use crate::reader::BitReader;
use core::ops::Range;

pub(crate) fn decode<'a>(
    data: &'a [u8],
    header: &'a Header,
    ctx: &mut DecoderContext<'a>,
) -> Result<()> {
    let mut reader = BitReader::new(data);
    let tiles = tile::parse(&mut reader, header)?;

    if tiles.is_empty() {
        bail!(TileError::Invalid);
    }

    ctx.reset(header, &tiles[0])?;

    let mut needed = Vec::with_capacity(header.component_infos.len());

    for tile in &tiles {
        needed.clear();
        needed.extend(
            header
                .component_infos
                .iter()
                .map(|info| needed_samples(tile, header, info)),
        );

        // Tiles that don't contribute any samples to the decoded region
        // can be skipped entirely.
        if needed.iter().all(|r| r.width() == 0 || r.height() == 0) {
            trace!("skipping tile {} outside of the decoded region", tile.idx);
            continue;
        }

        trace!(
            "tile {} rect [{},{} {}x{}]",
            tile.idx,
            tile.rect.x0,
            tile.rect.y0,
            tile.rect.width(),
            tile.rect.height(),
        );

        let progression_iterator = progression_iterator(tile, header)?;

        decode_tile(
            tile,
            header,
            progression_iterator,
            &needed,
            &mut ctx.tile_decode_context,
            &mut ctx.channel_data,
            &mut ctx.storage,
        )?;
    }

    // Note that this assumes that either all tiles have MCT or none of them.
    // In theory, only some could have it... But hopefully no such cursed
    // images exist!
    if tiles[0].mct {
        mct::apply_inverse(&mut ctx.channel_data, &tiles[0].component_infos, header)?;
    }

    apply_sign_shift(&mut ctx.channel_data, &header.component_infos);

    Ok(())
}

/// A decoder context for decoding JPEG2000 images.
#[derive(Default)]
pub struct DecoderContext<'a> {
    tile_decode_context: TileDecodeContext,
    /// The raw, decoded samples for each channel.
    pub(crate) channel_data: Vec<ComponentData>,
    storage: DecompositionStorage<'a>,
}

impl DecoderContext<'_> {
    fn reset(&mut self, header: &Header, initial_tile: &Tile<'_>) -> Result<()> {
        self.tile_decode_context.reset();
        self.storage.reset();

        self.channel_data.clear();
        let sample_count = (header.region.width() as usize)
            .checked_mul(header.region.height() as usize)
            .ok_or(ValidationError::ImageTooLarge)?;
        // TODO: SIMD Buffers should be reused across runs!
        for info in &initial_tile.component_infos {
            self.channel_data.push(ComponentData {
                container: SimdBuffer::zeros(sample_count),
                bit_depth: info.size_info.precision,
            });
        }

        Ok(())
    }
}

fn decode_tile<'a, 'b>(
    tile: &'b Tile<'a>,
    header: &Header,
    progression_iterator: Box<dyn Iterator<Item = ProgressionData> + '_>,
    needed: &[IntRect],
    tile_ctx: &mut TileDecodeContext,
    channel_data: &mut [ComponentData],
    storage: &mut DecompositionStorage<'a>,
) -> Result<()> {
    storage.reset();

    // This is the method that orchestrates all steps.

    // First, we build the decompositions, including their sub-bands, precincts
    // and code blocks.
    build::build(tile, storage, header.skipped_resolution_levels, needed)?;
    // Next, we parse the layers/segments for each code block.
    segment::parse(tile, progression_iterator, header, storage)?;

    // Interleave bitplane decoding, IDWT, and storage for each component tile
    // so we can reuse the coefficient and IDWT buffers.
    for (idx, component_info) in header.component_infos.iter().enumerate() {
        if idx > 0 {
            let count = storage.tile_decompositions[idx].coefficient_count;
            storage.coefficients[..count].fill(0.0);
        }
        // We then decode the bitplanes of each code block, yielding the
        // (possibly dequantized) coefficients of each code block.
        decode_component_tile_bit_planes(idx, tile, tile_ctx, storage, header)?;
        // Next, we apply the inverse discrete wavelet transform.
        idwt::apply(
            storage,
            tile_ctx,
            idx,
            header,
            component_info.wavelet_transform(),
        );
        // Finally, we store the raw samples for the tile area in the correct
        // location. Note that in case we have MCT, we are not applying it yet.
        // It will be applied in the very end once all tiles have been processed.
        // The reason we do this is that applying MCT requires access to the
        // data from _all_ components. If we didn't defer this until the end
        // we would have to collect the IDWT outputs of all components before
        // applying it. By not applying MCT here, we can get away with doing
        // IDWT and store on a per-component basis. Thus, we only need to
        // store one IDWT output at a time, allowing for better reuse of
        // allocations.
        store(
            tile,
            header,
            tile_ctx,
            &mut channel_data[idx],
            component_info,
            needed[idx],
        );
    }

    Ok(())
}

/// All decompositions for a single tile.
#[derive(Clone)]
pub(crate) struct TileDecompositions {
    pub(crate) first_ll_sub_band: usize,
    pub(crate) decompositions: Range<usize>,
    pub(crate) coefficient_count: usize,
}

impl TileDecompositions {
    pub(crate) fn sub_band_iter(
        &self,
        resolution: u8,
        decompositions: &[Decomposition],
    ) -> SubBandIter {
        let indices = if resolution == 0 {
            [
                self.first_ll_sub_band,
                self.first_ll_sub_band,
                self.first_ll_sub_band,
            ]
        } else {
            decompositions[self.decompositions.clone()][resolution as usize - 1].sub_bands
        };

        SubBandIter {
            next_idx: 0,
            indices,
            resolution,
        }
    }
}

#[derive(Clone)]
pub(crate) struct SubBandIter {
    resolution: u8,
    next_idx: usize,
    indices: [usize; 3],
}

impl Iterator for SubBandIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let value = if self.resolution == 0 {
            if self.next_idx > 0 {
                None
            } else {
                Some(self.indices[0])
            }
        } else if self.next_idx >= self.indices.len() {
            None
        } else {
            Some(self.indices[self.next_idx])
        };

        self.next_idx += 1;

        value
    }
}

/// A buffer so that we can reuse allocations for layers/code blocks/etc.
/// across different tiles.
#[derive(Default)]
pub(crate) struct DecompositionStorage<'a> {
    pub(crate) segments: Vec<Segment<'a>>,
    pub(crate) layers: Vec<Layer>,
    pub(crate) code_blocks: Vec<CodeBlock>,
    pub(crate) precincts: Vec<Precinct>,
    pub(crate) tag_tree_nodes: Vec<TagNode>,
    pub(crate) coefficients: Vec<f32>,
    pub(crate) sub_bands: Vec<SubBand>,
    pub(crate) decompositions: Vec<Decomposition>,
    pub(crate) tile_decompositions: Vec<TileDecompositions>,
}

impl DecompositionStorage<'_> {
    fn reset(&mut self) {
        self.segments.clear();
        self.layers.clear();
        self.code_blocks.clear();
        // No need to clear the coefficients, as they will be resized
        // and then overridden.
        // self.coefficients.clear();
        self.precincts.clear();
        self.sub_bands.clear();
        self.decompositions.clear();
        self.tile_decompositions.clear();
        self.tag_tree_nodes.clear();
    }
}

/// A reusable context used during the decoding of a single tile.
///
/// Some of the fields are temporary in nature and reset after moving on to the
/// next tile, some contain global state.
#[derive(Default)]
pub(crate) struct TileDecodeContext {
    /// A reusable buffer for the IDWT output.
    pub(crate) idwt_output: IDWTOutput,
    /// A scratch buffer used during IDWT.
    pub(crate) idwt_scratch_buffer: Vec<f32>,
    /// A reusable context for decoding code blocks.
    pub(crate) bit_plane_decode_context: BitPlaneDecodeContext,
    /// Reusable buffers for decoding bitplanes.
    pub(crate) bit_plane_decode_buffers: BitPlaneDecodeBuffers,
}

impl TileDecodeContext {
    fn reset(&mut self) {
        // This method doesn't do anything, just keeping it there in case
        // it's needed in the future.
        // Bitplane decode context and buffers will be reset in the
        // corresponding methods. IDWT output and scratch buffer will be
        // overridden on demand, so those don't need to be reset either.
    }
}

fn decode_component_tile_bit_planes<'a>(
    component_idx: usize,
    tile: &Tile<'a>,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage<'a>,
    header: &Header,
) -> Result<()> {
    let component_info = &tile.component_infos[component_idx];

    // Only decode the resolution levels we actually care about.
    for resolution in 0..component_info.num_resolution_levels() - header.skipped_resolution_levels {
        let tile_composition = &storage.tile_decompositions[component_idx];
        let sub_band_iter = tile_composition.sub_band_iter(resolution, &storage.decompositions);

        for sub_band_idx in sub_band_iter {
            decode_sub_band_bitplanes(
                sub_band_idx,
                resolution,
                component_info,
                tile_ctx,
                storage,
                header,
            )?;
        }
    }

    Ok(())
}

fn decode_sub_band_bitplanes(
    sub_band_idx: usize,
    resolution: u8,
    component_info: &ComponentInfo,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage<'_>,
    header: &Header,
) -> Result<()> {
    let sub_band = &storage.sub_bands[sub_band_idx];

    let quantised =
        component_info.quantization_info.quantization_style != QuantizationStyle::NoQuantization;
    let irreversible = component_info.wavelet_transform() == WaveletTransform::Irreversible97;
    let dequantization_step = {
        if !quantised {
            1.0
        } else {
            let (exponent, mantissa) =
                component_info.exponent_mantissa(sub_band.sub_band_type, resolution)?;

            let r_b = {
                let log_gain = match sub_band.sub_band_type {
                    SubBandType::LowLow => 0,
                    SubBandType::LowHigh => 1,
                    SubBandType::HighLow => 1,
                    SubBandType::HighHigh => 2,
                };

                component_info.size_info.precision as u16 + log_gain
            };

            crate::math::pow2i(r_b as i32 - exponent as i32) * (1.0 + (mantissa as f32) / 2048.0)
        }
    };

    let num_bitplanes = {
        let (exponent, _) = component_info.exponent_mantissa(sub_band.sub_band_type, resolution)?;
        // Equations (E-2) and (H-3).
        let num_bitplanes = (component_info.quantization_info.guard_bits as u16)
            .checked_add(exponent)
            .and_then(|x| x.checked_sub(1))
            .and_then(|x| x.checked_add(component_info.roi_shift as u16))
            .ok_or(DecodingError::InvalidBitplaneCount)?;

        if num_bitplanes > MAX_BITPLANE_COUNT as u16 {
            bail!(DecodingError::TooManyBitplanes);
        }

        num_bitplanes as u8
    };

    let window = sub_band.window;

    for precinct in sub_band
        .precincts
        .clone()
        .map(|idx| &storage.precincts[idx])
    {
        for code_block in precinct
            .code_blocks
            .clone()
            .map(|idx| &storage.code_blocks[idx])
        {
            // Only decode the code-blocks that contribute to the decoded
            // region.
            let area = code_block.rect.intersect(window);
            if area.width() == 0 || area.height() == 0 {
                continue;
            }

            bitplane::decode(
                code_block,
                sub_band.sub_band_type,
                num_bitplanes,
                &component_info.coding_style.parameters.code_block_style,
                tile_ctx,
                storage,
                header.strict,
            )?;

            // Turn the signs and magnitudes into singular coefficients and
            // copy the ones inside of the window into the sub-band.

            let x_offset = (area.x0 - window.x0) as usize;
            let y_offset = (area.y0 - window.y0) as usize;
            let skip_x = (area.x0 - code_block.rect.x0) as usize;
            let skip_y = (area.y0 - code_block.rect.y0) as usize;
            let width = area.width() as usize;

            let base_store = &mut storage.coefficients[sub_band.coefficients.clone()];
            let mut base_idx = y_offset * window.width() as usize + x_offset;

            for (coefficients, coefficient_states) in tile_ctx
                .bit_plane_decode_context
                .coefficient_rows()
                .skip(skip_y)
                .take(area.height() as usize)
            {
                let out_row = &mut base_store[base_idx..][..width];
                let coefficients = &coefficients[skip_x..];
                let coefficient_states = &coefficient_states[skip_x..];

                for ((output, coefficient), coefficient_state) in
                    out_row.iter_mut().zip(coefficients).zip(coefficient_states)
                {
                    *output = coefficient.reconstructed(
                        coefficient_state,
                        irreversible,
                        component_info.roi_shift,
                    );
                    *output *= dequantization_step;
                }

                base_idx += window.width() as usize;
            }
        }
    }

    Ok(())
}

fn apply_sign_shift(channel_data: &mut [ComponentData], component_infos: &[ComponentInfo]) {
    use crate::math::{Level, dispatch, f32x8};

    for (channel, component_info) in channel_data.iter_mut().zip(component_infos.iter()) {
        let offset = (1_u32 << (component_info.size_info.precision - 1)) as f32;
        dispatch!(Level::new(), simd => {
            let offset_v = f32x8::splat(simd, offset);
            for chunk in channel.container.as_chunks_mut::<8>().0 {
                let v = f32x8::from_slice(simd, chunk);
                (v + offset_v).store(chunk);
            }
        });
    }
}

/// Return the samples of the highest decoded resolution of a component tile
/// that contribute to the decoded region (in the coordinate system of the
/// resolution, see B-14). This may include a few samples more than
/// necessary, `store` only writes the ones inside of the region.
fn needed_samples(tile: &Tile<'_>, header: &Header, component_info: &ComponentInfo) -> IntRect {
    let resolution_tile = ResolutionTile::new(
        ComponentTile::new(tile, component_info),
        component_info.num_resolution_levels() - 1 - header.skipped_resolution_levels,
    );
    let rect = resolution_tile.rect;
    let size_data = &header.size_data;
    let region = header.region;

    // A sample at position `x` of the component covers the positions
    // `[scale * x / shrink, scale * (x + 1) / shrink)` of the output image
    // (see `store`). Return the samples covering `[start, end)`.
    let range = |start: u32, end: u32, scale: u8, shrink: u32, first: bool, x0: u32, x1: u32| {
        let (scale, shrink) = (u64::from(scale), u64::from(shrink));
        let lo = u64::from(start) * shrink / scale;
        let mut hi = (u64::from(end) * shrink).div_ceil(scale);
        // The first sample of the image is stretched to the image offset.
        if first {
            hi = u64::max(hi, u64::from(x0) + 1);
        }

        (
            u64::clamp(lo, u64::from(x0), u64::from(x1)) as u32,
            u64::clamp(hi, u64::from(x0), u64::from(x1)) as u32,
        )
    };

    let (x0, x1) = range(
        region.x0 + size_data.image_x_offset(),
        region.x1 + size_data.image_x_offset(),
        component_info.size_info.horizontal_resolution,
        size_data.x_shrink_factor,
        tile.rect.x0 == size_data.image_area_x_offset,
        rect.x0,
        rect.x1,
    );
    let (y0, y1) = range(
        region.y0 + size_data.image_y_offset(),
        region.y1 + size_data.image_y_offset(),
        component_info.size_info.vertical_resolution,
        size_data.y_shrink_factor,
        tile.rect.y0 == size_data.image_area_y_offset,
        rect.y0,
        rect.y1,
    );

    IntRect::from_ltrb(x0, y0, x1, y1)
}

fn store<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    tile_ctx: &mut TileDecodeContext,
    channel_data: &mut ComponentData,
    component_info: &ComponentInfo,
    needed: IntRect,
) {
    let idwt_output = &mut tile_ctx.idwt_output;

    // Resolution levels can be empty (B-14), in which case there is nothing
    // to store.
    if idwt_output.rect.width() == 0
        || idwt_output.rect.height() == 0
        || needed.width() == 0
        || needed.height() == 0
    {
        return;
    }

    let (scale_x, scale_y) = (
        component_info.size_info.horizontal_resolution,
        component_info.size_info.vertical_resolution,
    );

    let (image_x_offset, image_y_offset) = (
        header.size_data.image_x_offset(),
        header.size_data.image_y_offset(),
    );

    // The decoded region in the coordinate system of the reference grid,
    // reduced to the decoded resolution.
    let region = IntRect::from_ltrb(
        header.region.x0 + image_x_offset,
        header.region.y0 + image_y_offset,
        header.region.x1 + image_x_offset,
        header.region.y1 + image_y_offset,
    );
    let region_width = region.width() as usize;

    // The IDWT output covers the window of the highest decomposition level
    // that was computed, which includes all samples in `needed`.
    let input_width = idwt_output.rect.width() as usize;
    let input_index = |x: u32, y: u32| {
        (y - idwt_output.rect.y0) as usize * input_width + (x - idwt_output.rect.x0) as usize
    };

    if scale_x == 1 && scale_y == 1 {
        // If no sub-sampling, use a fast path where we copy rows of coefficients
        // at once.
        let area = needed.intersect(region);
        let width = area.width() as usize;

        if width == 0 {
            return;
        }

        for y in area.y0..area.y1 {
            let input_row = &idwt_output.coefficients[input_index(area.x0, y)..][..width];
            let output_row = &mut channel_data.container
                [(y - region.y0) as usize * region_width + (area.x0 - region.x0) as usize..]
                [..width];

            output_row.copy_from_slice(input_row);
        }
    } else {
        let x_min = u32::max(image_x_offset, region.x0);
        let y_min = u32::max(image_y_offset, region.y0);
        let x_max = u32::min(header.size_data.image_width() + image_x_offset, region.x1);
        let y_max = u32::min(header.size_data.image_height() + image_y_offset, region.y1);

        let x_shrink_factor = header.size_data.x_shrink_factor;
        let y_shrink_factor = header.size_data.y_shrink_factor;

        let resolution_tile = ResolutionTile::new(
            ComponentTile::new(tile, component_info),
            component_info.num_resolution_levels() - 1 - header.skipped_resolution_levels,
        );

        // Otherwise, copy sample by sample.
        let first_x = (tile.rect.x0 == header.size_data.image_area_x_offset)
            .then_some(resolution_tile.rect.x0);
        let first_y = (tile.rect.y0 == header.size_data.image_area_y_offset)
            .then_some(resolution_tile.rect.y0);

        for y in needed.y0..needed.y1 {
            let reference_grid_y = if first_y == Some(y) {
                image_y_offset
            } else {
                (scale_y as u32 * y) / y_shrink_factor
            };
            let y_end = (scale_y as u32 * (y + 1)) / y_shrink_factor;

            for x in needed.x0..needed.x1 {
                let reference_grid_x = if first_x == Some(x) {
                    image_x_offset
                } else {
                    (scale_x as u32 * x) / x_shrink_factor
                };
                let x_end = (scale_x as u32 * (x + 1)) / x_shrink_factor;

                let sample = idwt_output.coefficients[input_index(x, y)];

                for x_position in u32::max(reference_grid_x, x_min)..u32::min(x_end, x_max) {
                    for y_position in u32::max(reference_grid_y, y_min)..u32::min(y_end, y_max) {
                        let pos = (y_position - region.y0) as usize * region_width
                            + (x_position - region.x0) as usize;

                        channel_data.container[pos] = sample;
                    }
                }
            }
        }
    }
}
