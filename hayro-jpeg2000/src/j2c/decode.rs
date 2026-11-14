use alloc::boxed::Box;
use alloc::vec::Vec;

use super::bitplane::{BitPlaneDecodeBuffers, BitPlaneDecodeContext};
use super::build::{CodeBlock, Decomposition, Layer, Precinct, Segment, SubBand, SubBandType};
use super::codestream::{ComponentInfo, Header, QuantizationStyle, WaveletTransform};
use super::idwt::IDWTOutput;
use super::progression::{ProgressionData, progression_iterator};
use super::tag_tree::TagNode;
use super::tile::{ComponentTile, ResolutionTile, Tile};
use super::{ComponentData, bitplane, build, idwt, mct, segment, tile};
use crate::cancel::Cancel;
use crate::error::{DecodingError, Result, TileError, ValidationError, bail};
use crate::j2c::segment::MAX_BITPLANE_COUNT;
use crate::math::SimdBuffer;
use crate::reader::BitReader;
use core::ops::Range;

pub(crate) fn decode<'a>(
    data: &'a [u8],
    header: &'a Header,
    ctx: &mut DecoderContext<'a>,
    cancel: Cancel<'_>,
) -> Result<()> {
    let mut reader = BitReader::new(data);
    let tiles = tile::parse(&mut reader, header)?;

    if tiles.is_empty() {
        bail!(TileError::Invalid);
    }

    ctx.reset(header, &tiles[0], cancel)?;

    for tile in &tiles {
        cancel.check()?;

        let progression_iterator = progression_iterator(tile, header)?;

        decode_tile(
            tile,
            header,
            progression_iterator,
            &mut ctx.tile_decode_context,
            &mut ctx.channel_data,
            &mut ctx.storage,
            cancel,
        )?;
    }

    // Assumes either all tiles or none use MCT.
    if tiles[0].mct {
        mct::apply_inverse(
            &mut ctx.channel_data,
            &tiles[0].component_infos,
            header,
            cancel,
        )?;
    }

    apply_sign_shift(&mut ctx.channel_data, &header.component_infos, cancel)?;

    Ok(())
}

/// A decoder context for decoding JPEG2000 images.
#[derive(Default)]
pub struct DecoderContext<'a> {
    tile_decode_context: TileDecodeContext,
    pub(crate) channel_data: Vec<ComponentData>,
    storage: DecompositionStorage<'a>,
}

impl DecoderContext<'_> {
    fn reset(
        &mut self,
        header: &Header,
        initial_tile: &Tile<'_>,
        cancel: Cancel<'_>,
    ) -> Result<()> {
        self.tile_decode_context.reset();
        self.storage.reset();

        self.channel_data.clear();
        let sample_count = (header.size_data.image_width() as usize)
            .checked_mul(header.size_data.image_height() as usize)
            .ok_or(ValidationError::ImageTooLarge)?;
        // TODO: SIMD Buffers should be reused across runs!
        for info in &initial_tile.component_infos {
            cancel.check()?;

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
    tile_ctx: &mut TileDecodeContext,
    channel_data: &mut [ComponentData],
    storage: &mut DecompositionStorage<'a>,
    cancel: Cancel<'_>,
) -> Result<()> {
    storage.reset();

    build::build(tile, storage, header.skipped_resolution_levels, cancel)?;
    segment::parse(tile, progression_iterator, header, storage, cancel)?;

    for (idx, component_info) in header.component_infos.iter().enumerate() {
        cancel.check()?;

        if idx > 0 {
            let count = storage.tile_decompositions[idx].coefficient_count;
            storage.coefficients[..count].fill(0.0);
        }
        decode_component_tile_bit_planes(idx, tile, tile_ctx, storage, header, cancel)?;
        idwt::apply(
            storage,
            tile_ctx,
            idx,
            header,
            component_info.wavelet_transform(),
            cancel,
        )?;
        // MCT is deferred until all tiles are stored so IDWT buffers can be reused per component.
        store(
            tile,
            header,
            tile_ctx,
            &mut channel_data[idx],
            component_info,
            cancel,
        )?;
    }

    Ok(())
}

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
        self.precincts.clear();
        self.sub_bands.clear();
        self.decompositions.clear();
        self.tile_decompositions.clear();
        self.tag_tree_nodes.clear();
    }
}

#[derive(Default)]
pub(crate) struct TileDecodeContext {
    pub(crate) idwt_output: IDWTOutput,
    pub(crate) idwt_scratch_buffer: Vec<f32>,
    pub(crate) bit_plane_decode_context: BitPlaneDecodeContext,
    pub(crate) bit_plane_decode_buffers: BitPlaneDecodeBuffers,
}

impl TileDecodeContext {
    fn reset(&mut self) {}
}

fn decode_component_tile_bit_planes<'a>(
    component_idx: usize,
    tile: &Tile<'a>,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage<'a>,
    header: &Header,
    cancel: Cancel<'_>,
) -> Result<()> {
    let component_info = &tile.component_infos[component_idx];

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
                cancel,
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
    cancel: Cancel<'_>,
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
            cancel.check()?;

            bitplane::decode(
                code_block,
                sub_band.sub_band_type,
                num_bitplanes,
                &component_info.coding_style.parameters.code_block_style,
                tile_ctx,
                storage,
                header.strict,
            )?;

            let x_offset = code_block.rect.x0 - sub_band.rect.x0;
            let y_offset = code_block.rect.y0 - sub_band.rect.y0;

            let base_store = &mut storage.coefficients[sub_band.coefficients.clone()];
            let mut base_idx = (y_offset * sub_band.rect.width()) as usize + x_offset as usize;

            for (coefficients, coefficient_states) in
                tile_ctx.bit_plane_decode_context.coefficient_rows()
            {
                let out_row = &mut base_store[base_idx..];

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

                base_idx += sub_band.rect.width() as usize;
            }
        }
    }

    Ok(())
}

fn apply_sign_shift(
    channel_data: &mut [ComponentData],
    component_infos: &[ComponentInfo],
    cancel: Cancel<'_>,
) -> Result<()> {
    use crate::math::{Level, dispatch, f32x8};

    for (channel, component_info) in channel_data.iter_mut().zip(component_infos.iter()) {
        cancel.check()?;

        let offset = (1_u32 << (component_info.size_info.precision - 1)) as f32;
        dispatch!(Level::new(), simd => {
            let offset_v = f32x8::splat(simd, offset);
            for chunk in channel.container.as_chunks_mut::<8>().0 {
                let v = f32x8::from_slice(simd, chunk);
                (v + offset_v).store(chunk);
            }
        });
    }

    Ok(())
}

fn store<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    tile_ctx: &mut TileDecodeContext,
    channel_data: &mut ComponentData,
    component_info: &ComponentInfo,
    cancel: Cancel<'_>,
) -> Result<()> {
    let idwt_output = &mut tile_ctx.idwt_output;

    let component_tile = ComponentTile::new(tile, component_info);
    let resolution_tile = ResolutionTile::new(
        component_tile,
        component_info.num_resolution_levels() - 1 - header.skipped_resolution_levels,
    );

    if idwt_output.rect.width() == 0 || idwt_output.rect.height() == 0 {
        return Ok(());
    }

    let (scale_x, scale_y) = (
        component_info.size_info.horizontal_resolution,
        component_info.size_info.vertical_resolution,
    );

    let (image_x_offset, image_y_offset) = (
        header.size_data.image_x_offset(),
        header.size_data.image_y_offset(),
    );

    if scale_x == 1 && scale_y == 1 {
        let skip_x = image_x_offset.saturating_sub(idwt_output.rect.x0);
        let skip_y = image_y_offset.saturating_sub(idwt_output.rect.y0);

        let input_row_iter = idwt_output
            .coefficients
            .chunks_exact(idwt_output.rect.width() as usize)
            .skip(skip_y as usize)
            .take(idwt_output.rect.height() as usize);

        let output_row_iter = channel_data
            .container
            .chunks_exact_mut(header.size_data.image_width() as usize)
            .skip(resolution_tile.rect.y0.saturating_sub(image_y_offset) as usize);

        for (input_row, output_row) in input_row_iter.zip(output_row_iter) {
            cancel.check()?;

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

        let first_x = (tile.rect.x0 == header.size_data.image_area_x_offset)
            .then_some(resolution_tile.rect.x0);
        let first_y = (tile.rect.y0 == header.size_data.image_area_y_offset)
            .then_some(resolution_tile.rect.y0);

        for y in resolution_tile.rect.y0..resolution_tile.rect.y1 {
            cancel.check()?;

            let relative_y = (y - idwt_output.rect.y0) as usize;
            let reference_grid_y = if first_y == Some(y) {
                image_y_offset
            } else {
                (scale_y as u32 * y) / y_shrink_factor
            };
            let y_end = (scale_y as u32 * (y + 1)) / y_shrink_factor;

            for x in resolution_tile.rect.x0..resolution_tile.rect.x1 {
                let relative_x = (x - idwt_output.rect.x0) as usize;
                let reference_grid_x = if first_x == Some(x) {
                    image_x_offset
                } else {
                    (scale_x as u32 * x) / x_shrink_factor
                };
                let x_end = (scale_x as u32 * (x + 1)) / x_shrink_factor;

                let sample = idwt_output.coefficients
                    [relative_y * idwt_output.rect.width() as usize + relative_x];

                for x_position in u32::max(reference_grid_x, image_x_offset)
                    ..u32::min(x_end, image_width + image_x_offset)
                {
                    for y_position in u32::max(reference_grid_y, image_y_offset)
                        ..u32::min(y_end, image_height + image_y_offset)
                    {
                        let pos = (y_position - image_y_offset) as usize * image_width as usize
                            + (x_position - image_x_offset) as usize;

                        channel_data.container[pos] = sample;
                    }
                }
            }
        }
    }

    Ok(())
}
