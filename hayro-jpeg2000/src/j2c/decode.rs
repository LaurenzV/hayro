//! Decoding JPEG2000 code streams.
//!
//! This is the "core" module of the crate that orchestrates all
//! stages in such a way that a given codestream is decoded into its
//! component channels.

use super::bitplane::{BitPlaneDecodeBuffers, BitPlaneDecodeContext};
use super::build::{CodeBlock, Decomposition, Layer, Precinct, Segment, SubBand, SubBandType};
use super::codestream::{ComponentInfo, Header, ProgressionOrder, QuantizationStyle};
use super::idwt::IDWTOutput;
use super::progression::{
    IteratorInput, ProgressionData, component_position_resolution_layer_progression,
    layer_resolution_component_position_progression,
    position_component_resolution_layer_progression,
    resolution_layer_component_position_progression,
    resolution_position_component_layer_progression,
};
use super::tag_tree::TagNode;
use super::tile::{ComponentTile, ResolutionTile, Tile};
use super::{ComponentData, bitplane, build, idwt, mct, segment, tile};
use crate::j2c::segment::MAX_BITPLANE_COUNT;
use crate::reader::BitReader;
use log::trace;
use std::ops::Range;

pub(crate) fn decode(data: &[u8], header: &Header<'_>) -> Result<Vec<ComponentData>, &'static str> {
    let mut reader = BitReader::new(data);
    let tiles = tile::parse(&mut reader, header)?;

    if tiles.is_empty() {
        return Err("the image doesn't contain any tiles");
    }

    let mut tile_ctx = TileDecodeContext::new(header, &tiles[0]);
    let mut storage = DecompositionStorage::default();

    for tile in tiles.iter() {
        trace!(
            "tile {} rect [{},{} {}x{}]",
            tile.idx,
            tile.rect.x0,
            tile.rect.y0,
            tile.rect.width(),
            tile.rect.height(),
        );

        let iter_input = IteratorInput::new(tile);

        let progression_iterator: Box<dyn Iterator<Item = ProgressionData>> =
            match tile.progression_order {
                ProgressionOrder::LayerResolutionComponentPosition => {
                    Box::new(layer_resolution_component_position_progression(iter_input))
                }
                ProgressionOrder::ResolutionLayerComponentPosition => {
                    Box::new(resolution_layer_component_position_progression(iter_input))
                }
                ProgressionOrder::ResolutionPositionComponentLayer => Box::new(
                    resolution_position_component_layer_progression(iter_input)
                        .ok_or("failed to build progression iterator")?,
                ),
                ProgressionOrder::PositionComponentResolutionLayer => Box::new(
                    position_component_resolution_layer_progression(iter_input)
                        .ok_or("failed to build progression iterator")?,
                ),
                ProgressionOrder::ComponentPositionResolutionLayer => Box::new(
                    component_position_resolution_layer_progression(iter_input)
                        .ok_or("failed to build progression iterator")?,
                ),
            };

        decode_tile(
            tile,
            header,
            progression_iterator,
            &mut tile_ctx,
            &mut storage,
        )?;
    }

    // Note that this assumes that either all tiles have MCT or none of them.
    // In theory, only some could have it... But hopefully no such cursed
    // images exist!
    if tile_ctx.tile.mct {
        mct::apply_inverse(&mut tile_ctx, header)?;
        apply_sign_shift(&mut tile_ctx, &header.component_infos);
    }

    Ok(tile_ctx.channel_data)
}

fn decode_tile<'a>(
    tile: &'a Tile<'a>,
    header: &Header<'_>,
    progression_iterator: Box<dyn Iterator<Item = ProgressionData> + '_>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage<'a>,
) -> Result<(), &'static str> {
    tile_ctx.set_tile(tile);
    storage.reset();

    // This is the method that orchestrates all steps.

    // First, we build the decompositions, including their sub-bands, precincts
    // and code blocks.
    build::build(tile, tile_ctx, storage)?;
    // Next, we parse the layers/segments for each code block.
    segment::parse(tile, progression_iterator, tile_ctx, header, storage)?;
    // We then decode the bitplanes of each code block, yielding the
    // (possibly dequantized) coefficients of each code block.
    decode_component_tile_bit_planes(tile, tile_ctx, storage, header)?;

    // Unlike before, we interleave the apply_idwt and store stages
    // for each component tile so we can reuse allocations better.
    for (idx, component_info) in header.component_infos.iter().enumerate() {
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
        store(tile, header, tile_ctx, component_info, idx);
    }

    Ok(())
}

/// All decompositions for a single tile.
#[derive(Clone)]
pub(crate) struct TileDecompositions {
    pub(crate) first_ll_sub_band: usize,
    pub(crate) decompositions: Range<usize>,
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
pub(crate) struct TileDecodeContext<'a> {
    /// The tile that we are currently decoding.
    pub(crate) tile: &'a Tile<'a>,
    /// A reusable buffer for the IDWT output.
    pub(crate) idwt_output: IDWTOutput,
    /// A scratch buffer used during IDWT.
    pub(crate) idwt_scratch_buffer: Vec<f32>,
    /// A reusable context for decoding code blocks.
    pub(crate) bit_plane_decode_context: BitPlaneDecodeContext,
    /// Reusable buffers for decoding bitplanes.
    pub(crate) bit_plane_decode_buffers: BitPlaneDecodeBuffers,
    /// The raw, decoded samples for each channel.
    pub(crate) channel_data: Vec<ComponentData>,
}

impl<'a> TileDecodeContext<'a> {
    fn new(header: &Header<'_>, initial_tile: &'a Tile<'a>) -> Self {
        let mut channel_data = vec![];

        for info in &initial_tile.component_infos {
            channel_data.push(ComponentData {
                container: vec![
                    0.0;
                    header.size_data.image_width() as usize
                        * header.size_data.image_height() as usize
                ],
                bit_depth: info.size_info.precision,
            });
        }

        Self {
            tile: initial_tile,
            idwt_scratch_buffer: vec![],
            idwt_output: IDWTOutput::dummy(),
            bit_plane_decode_context: BitPlaneDecodeContext::default(),
            bit_plane_decode_buffers: BitPlaneDecodeBuffers::default(),
            channel_data,
        }
    }

    fn set_tile(&mut self, tile: &'a Tile<'a>) {
        // This is all that is needed when advancing to a new tile.
        // The other fields will be resetted in due course as needed.
        self.tile = tile;
    }
}

fn decode_component_tile_bit_planes<'a>(
    tile: &'a Tile<'a>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage<'a>,
    header: &Header<'_>,
) -> Result<(), &'static str> {
    for (tile_decompositions_idx, component_info) in tile.component_infos.iter().enumerate() {
        // Only decode the resolution levels we actually care about.
        for resolution in
            0..component_info.num_resolution_levels() - header.skipped_resolution_levels
        {
            let tile_composition = &storage.tile_decompositions[tile_decompositions_idx];
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
    }

    Ok(())
}

fn decode_sub_band_bitplanes(
    sub_band_idx: usize,
    resolution: u8,
    component_info: &ComponentInfo,
    tile_ctx: &mut TileDecodeContext<'_>,
    storage: &mut DecompositionStorage<'_>,
    header: &Header<'_>,
) -> Result<(), &'static str> {
    let sub_band = &storage.sub_bands[sub_band_idx];

    let dequantization_step = {
        if component_info.quantization_info.quantization_style == QuantizationStyle::NoQuantization
        {
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

            2.0_f32.powf(r_b as f32 - exponent as f32)
                * (1.0 + (mantissa as f32) / (2_u32.pow(11) as f32))
        }
    };

    let num_bitplanes = {
        let (exponent, _) = component_info.exponent_mantissa(sub_band.sub_band_type, resolution)?;
        // Equation (E-2)
        let num_bitplanes = (component_info.quantization_info.guard_bits as u16)
            .checked_add(exponent)
            .and_then(|x| x.checked_sub(1))
            .ok_or("invalid number of bitplanes")?;

        if num_bitplanes > MAX_BITPLANE_COUNT as u16 {
            return Err("number of bitplanes is too large");
        }

        num_bitplanes as u8
    };

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
            // copy them into the sub-band.

            let x_offset = code_block.rect.x0 - sub_band.rect.x0;
            let y_offset = code_block.rect.y0 - sub_band.rect.y0;

            let base_store = &mut storage.coefficients[sub_band.coefficients.clone()];
            let mut base_idx = (y_offset * sub_band.rect.width()) as usize + x_offset as usize;

            for coefficients in tile_ctx.bit_plane_decode_context.coefficient_rows() {
                let out_row = &mut base_store[base_idx..];

                for (output, coefficient) in out_row.iter_mut().zip(coefficients.iter().copied()) {
                    *output = coefficient.get() as f32;
                    *output *= dequantization_step;
                }

                base_idx += sub_band.rect.width() as usize;
            }
        }
    }

    Ok(())
}

fn apply_sign_shift(tile_ctx: &mut TileDecodeContext<'_>, component_infos: &[ComponentInfo]) {
    for (channel_data, component_info) in
        tile_ctx.channel_data.iter_mut().zip(component_infos.iter())
    {
        for sample in &mut channel_data.container {
            *sample += (1_u32 << (component_info.size_info.precision - 1)) as f32;
        }
    }
}

fn store<'a>(
    tile: &'a Tile<'a>,
    header: &Header<'_>,
    tile_ctx: &mut TileDecodeContext<'a>,
    component_info: &ComponentInfo,
    component_idx: usize,
) {
    let channel_data = &mut tile_ctx.channel_data[component_idx];
    let idwt_output = &mut tile_ctx.idwt_output;

    let component_tile = ComponentTile::new(tile, component_info);
    let resolution_tile = ResolutionTile::new(
        component_tile,
        component_info.num_resolution_levels() - 1 - header.skipped_resolution_levels,
    );

    // If we have MCT, the sign shift needs to be applied after the
    // MCT transform. We take care of that in the main decode method.
    // Otherwise, we might as well just apply it now.
    if !tile.mct {
        for sample in idwt_output.coefficients.iter_mut() {
            *sample += (1_u32 << (component_info.size_info.precision - 1)) as f32;
        }
    }

    let (scale_x, scale_y) = (
        component_info.size_info.horizontal_resolution,
        component_info.size_info.vertical_resolution,
    );

    let (image_x_offset, image_y_offset) = (
        header.size_data.image_area_x_offset,
        header.size_data.image_area_y_offset,
    );

    if scale_x == 1 && scale_y == 1 {
        // If no sub-sampling, use a fast path where we copy rows of coefficients
        // at once.

        // The rect of the IDWT output corresponds to the rect of the highest
        // decomposition level of the tile, which is usually not 1:1 aligned
        // with the actual tile rectangle. We also need to account for the
        // offset of the reference grid.

        let skip_x = image_x_offset.saturating_sub(idwt_output.rect.x0);
        let skip_y = image_y_offset.saturating_sub(idwt_output.rect.y0);

        let input_row_iter = idwt_output
            .coefficients
            .chunks_exact(idwt_output.total_width() as usize)
            .map(|s| &s[idwt_output.padding.left..][..idwt_output.rect.width() as usize])
            .skip(skip_y as usize + idwt_output.padding.top)
            .take(idwt_output.rect.height() as usize);

        let output_row_iter = channel_data
            .container
            .chunks_exact_mut(header.size_data.image_width() as usize)
            .skip(resolution_tile.rect.y0.saturating_sub(image_y_offset) as usize);

        for (input_row, output_row) in input_row_iter.zip(output_row_iter) {
            let input_row = &input_row[skip_x as usize..];
            let output_row = &mut output_row
                [resolution_tile.rect.x0.saturating_sub(image_x_offset) as usize..]
                [..input_row.len()];

            output_row.copy_from_slice(input_row);
        }
    } else {
        let image_width = header.size_data.image_width();
        let image_height = header.size_data.image_height();

        let x_shrink_factor = header.size_data.x_shrink_factor;
        let y_shrink_factor = header.size_data.y_shrink_factor;

        let x_offset = header
            .size_data
            .image_area_x_offset
            .div_ceil(x_shrink_factor);
        let y_offset = header
            .size_data
            .image_area_y_offset
            .div_ceil(y_shrink_factor);

        // Otherwise, copy sample by sample.
        for y in resolution_tile.rect.y0..resolution_tile.rect.y1 {
            let relative_y = (y - component_tile.rect.y0) as usize;
            let reference_grid_y = (scale_y as u32 * y) / y_shrink_factor;

            for x in resolution_tile.rect.x0..resolution_tile.rect.x1 {
                let relative_x = (x - component_tile.rect.x0) as usize;
                let reference_grid_x = (scale_x as u32 * x) / x_shrink_factor;

                let sample = idwt_output.coefficients[(relative_y + idwt_output.padding.top)
                    * idwt_output.total_width() as usize
                    + relative_x
                    + idwt_output.padding.left];

                for x_position in u32::max(reference_grid_x, x_offset)
                    ..u32::min(reference_grid_x + scale_x as u32, image_width + x_offset)
                {
                    for y_position in u32::max(reference_grid_y, y_offset)
                        ..u32::min(reference_grid_y + scale_y as u32, image_height + y_offset)
                    {
                        let pos = (y_position - y_offset) as usize * image_width as usize
                            + (x_position - x_offset) as usize;

                        channel_data.container[pos] = sample;
                    }
                }
            }
        }
    }
}
