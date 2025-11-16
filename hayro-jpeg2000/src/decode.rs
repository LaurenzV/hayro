//! Decoding JPEG2000 code streams.
//!
//! This is the "core" module of the crate that orchestrages all
//! stages in such a way that a given codestream is decoded into its
//! component channels.

use crate::bitmap::ChannelData;
use crate::bitplane::CodeBlockDecodeContext;
use crate::codestream::markers::{EPH, SOP};
use crate::codestream::{
    ComponentInfo, Header, ProgressionOrder, QuantizationStyle, ReaderExt, WaveletTransform,
};
use crate::idwt::IDWTOutput;
use crate::progression::{
    IteratorInput, ProgressionData, build_component_position_resolution_layer_sequence,
    build_layer_resolution_component_position_sequence,
    build_position_component_resolution_layer_sequence,
    build_resolution_layer_component_position_sequence,
    build_resolution_position_component_layer_sequence,
};
use crate::rect::IntRect;
use crate::tag_tree::TagTree;
use crate::tile::{ComponentTile, ResolutionTile, Tile};
use crate::{bitplane, idwt, tile};
use hayro_common::bit::BitReader;
use hayro_common::byte::Reader;
use log::{trace, warn};
use std::iter;
use std::ops::Range;

pub(crate) fn decode(data: &[u8], header: &Header) -> Result<Vec<ChannelData>, &'static str> {
    let mut reader = Reader::new(data);
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

        match tile.progression_order {
            ProgressionOrder::LayerResolutionComponentPosition => {
                let iterator = build_layer_resolution_component_position_sequence(&iter_input);
                decode_tile(
                    tile,
                    header,
                    iterator.into_iter(),
                    &mut tile_ctx,
                    &mut storage,
                )?
            }
            ProgressionOrder::ResolutionLayerComponentPosition => {
                let iterator = build_resolution_layer_component_position_sequence(&iter_input);
                decode_tile(
                    tile,
                    header,
                    iterator.into_iter(),
                    &mut tile_ctx,
                    &mut storage,
                )?
            }
            ProgressionOrder::ResolutionPositionComponentLayer => {
                let iterator = build_resolution_position_component_layer_sequence(&iter_input);
                decode_tile(
                    tile,
                    header,
                    iterator.into_iter(),
                    &mut tile_ctx,
                    &mut storage,
                )?
            }
            ProgressionOrder::PositionComponentResolutionLayer => {
                let iterator = build_position_component_resolution_layer_sequence(&iter_input);
                decode_tile(
                    tile,
                    header,
                    iterator.into_iter(),
                    &mut tile_ctx,
                    &mut storage,
                )?
            }
            ProgressionOrder::ComponentPositionResolutionLayer => {
                let iterator = build_component_position_resolution_layer_sequence(&iter_input);
                decode_tile(
                    tile,
                    header,
                    iterator.into_iter(),
                    &mut tile_ctx,
                    &mut storage,
                )?
            }
        };
    }

    Ok(tile_ctx.channel_data)
}

fn decode_tile<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    progression_iterator: impl Iterator<Item = ProgressionData>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage<'a>,
) -> Result<(), &'static str> {
    tile_ctx.reset(tile);
    storage.reset();

    // This is the method that orchestrates all steps.

    // First, we build the decompositions, including their sub-bands, precincts
    // and code blocks.
    build_decompositions(tile, tile_ctx, storage)?;
    // Next, we parse the layer data for each code block.
    get_code_block_data(tile, progression_iterator, tile_ctx, storage)?;
    // We then decode the bitplanes of each code block, yielding the
    // (possibly dequantized) coefficients of each code block.
    decode_bitplanes(tile, tile_ctx, storage)?;
    // Next, we apply the inverse discrete wavelet transform.
    apply_idwt(tile, tile_ctx, storage)?;
    // If applicable, we apply the multi-component transform.
    apply_mct(tile_ctx);
    // Finally, we store the raw samples for the tile area in the correct
    // location.
    store(tile, header, tile_ctx);

    Ok(())
}

/// All decompositions for a single tile.
#[derive(Clone)]
struct TileDecompositions {
    first_ll_sub_band: usize,
    decompositions: Range<usize>,
}

impl TileDecompositions {
    fn sub_band_iter(&self, resolution: u16, decompositions: &[Decomposition]) -> SubBandIter {
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
struct SubBandIter {
    resolution: u16,
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

pub(crate) struct Decomposition {
    /// In the order low-high, high-low and high-high.
    pub(crate) sub_bands: [usize; 3],
    /// The rectangle of the decomposition.
    pub(crate) rect: IntRect,
}

#[derive(Clone)]
pub(crate) struct SubBand {
    pub(crate) sub_band_type: SubBandType,
    pub(crate) rect: IntRect,
    pub(crate) precincts: Range<usize>,
    // TODO: Store in allocation storage.
    pub(crate) coefficients: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubBandType {
    LowLow,
    LowHigh,
    HighLow,
    HighHigh,
}

#[derive(Clone)]
pub(crate) struct Precinct {
    code_blocks: Range<usize>,
    code_inclusion_tree: TagTree,
    zero_bitplane_tree: TagTree,
}

#[derive(Clone)]
pub(crate) struct CodeBlock {
    pub(crate) rect: IntRect,
    pub(crate) x_idx: u32,
    pub(crate) y_idx: u32,
    pub(crate) layers: Range<usize>,
    pub(crate) has_been_included: bool,
    pub(crate) missing_bit_planes: u8,
    pub(crate) number_of_coding_passes: u32,
    pub(crate) l_block: u32,
    pub(crate) non_empty_layer_count: u32,
}

pub(crate) struct Segment<'a> {
    pub(crate) idx: u32,
    pub(crate) coding_pases: u32,
    pub(crate) data_length: u32,
    pub(crate) data: &'a [u8],
}

#[derive(Clone)]
pub(crate) struct Layer {
    pub(crate) segments: Option<Range<usize>>,
}

/// A buffer so that we can reuse allocations for layers/code blocks/etc.
/// across different tiles.
#[derive(Default)]
struct DecompositionStorage<'a> {
    segments: Vec<Segment<'a>>,
    layers: Vec<Layer>,
    code_blocks: Vec<CodeBlock>,
    precincts: Vec<Precinct>,
    coefficients: Vec<f32>,
    sub_bands: Vec<SubBand>,
    decompositions: Vec<Decomposition>,
    tile_decompositions: Vec<TileDecompositions>,
}

impl DecompositionStorage<'_> {
    fn reset(&mut self) {
        self.segments.clear();
        self.layers.clear();
        self.code_blocks.clear();
        self.coefficients.clear();
        self.precincts.clear();
        self.sub_bands.clear();
        self.decompositions.clear();
        self.tile_decompositions.clear();
    }
}

/// A reusable context used during the decoding of a single tile.
///
/// Some of the fields are temporary in nature and reset after moving on to the
/// next tile, some contain global state.
pub(crate) struct TileDecodeContext<'a> {
    /// The tile that we are currently decoding.
    pub(crate) tile: &'a Tile<'a>,
    /// The outputs of the IDWT operations of each component of the tile
    /// we are currently processing.
    pub(crate) idwt_outputs: Vec<IDWTOutput>,
    /// A reusable context for decoding code blocks.
    pub(crate) code_block_decode_context: CodeBlockDecodeContext,
    /// The raw, decoded samples for each channel.
    pub(crate) channel_data: Vec<ChannelData>,
}

impl<'a> TileDecodeContext<'a> {
    fn new(header: &Header, initial_tile: &'a Tile<'a>) -> Self {
        let mut channel_data = vec![];

        for info in &initial_tile.component_infos {
            channel_data.push(ChannelData {
                container: vec![
                    0.0;
                    (header.size_data.image_width() * header.size_data.image_height())
                        as usize
                ],
                // Will be set later on, because that data only exists in the
                // metadata of the JP2 file, not the actual code stream.
                is_alpha: false,
                bit_depth: info.size_info.precision,
            })
        }

        Self {
            tile: initial_tile,
            idwt_outputs: vec![],
            code_block_decode_context: Default::default(),
            channel_data,
        }
    }

    fn reset(&mut self, tile: &'a Tile<'a>) {
        self.tile = tile;
        self.idwt_outputs.clear();
        // Code-block decode context will be resetted before being used, can't
        // do it here because we need data for a code block.
        // Channel data should not be resetted because it's global.
    }
}

fn build_decompositions(
    tile: &Tile,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage,
) -> Result<(), &'static str> {
    for (component_idx, component_tile) in tile.component_tiles().enumerate() {
        // TODO: IMprove this
        let mut ll_sub_band = None;

        let start = storage.decompositions.len();

        for resolution_tile in component_tile.resolution_tiles() {
            let resolution = resolution_tile.resolution;

            if resolution == 0 {
                let sub_band_rect = resolution_tile.sub_band_rect(SubBandType::LowLow);

                trace!("making nLL for component {}", component_idx);
                trace!(
                    "Sub-band rect: [{},{} {}x{}], ll rect [{},{} {}x{}]",
                    sub_band_rect.x0,
                    sub_band_rect.y0,
                    sub_band_rect.width(),
                    sub_band_rect.height(),
                    resolution_tile.rect.x0,
                    resolution_tile.rect.y0,
                    resolution_tile.rect.width(),
                    resolution_tile.rect.height(),
                );
                let precincts =
                    build_precincts(&resolution_tile, sub_band_rect, tile_ctx, storage)?;

                ll_sub_band = Some(SubBand {
                    sub_band_type: SubBandType::LowLow,
                    rect: sub_band_rect,
                    precincts,
                    coefficients: vec![
                        0.0;
                        (sub_band_rect.width() * sub_band_rect.height()) as usize
                    ],
                })
            } else {
                let mut build_sub_band =
                    |sub_band_type: SubBandType, storage: &mut DecompositionStorage| {
                        let sub_band_rect = resolution_tile.sub_band_rect(sub_band_type);

                        let precincts =
                            build_precincts(&resolution_tile, sub_band_rect, tile_ctx, storage)?;

                        let idx = storage.sub_bands.len();
                        storage.sub_bands.push(SubBand {
                            sub_band_type,
                            rect: sub_band_rect,
                            precincts: precincts.clone(),
                            coefficients: vec![
                                0.0;
                                (sub_band_rect.width() * sub_band_rect.height())
                                    as usize
                            ],
                        });

                        Ok(idx)
                    };

                let decomposition = Decomposition {
                    sub_bands: [
                        build_sub_band(SubBandType::HighLow, storage)?,
                        build_sub_band(SubBandType::LowHigh, storage)?,
                        build_sub_band(SubBandType::HighHigh, storage)?,
                    ],
                    rect: resolution_tile.rect,
                };

                storage.decompositions.push(decomposition);
            }
        }

        let end = storage.decompositions.len();
        let first_ll_sub_band = storage.sub_bands.len();
        storage.sub_bands.push(ll_sub_band.unwrap());

        storage.tile_decompositions.push(TileDecompositions {
            decompositions: start..end,
            first_ll_sub_band,
        });
    }

    Ok(())
}

fn build_precincts(
    resolution_tile: &ResolutionTile,
    sub_band_rect: IntRect,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage,
) -> Result<Range<usize>, &'static str> {
    let num_precincts_y = resolution_tile.num_precincts_y();
    let num_precincts_x = resolution_tile.num_precincts_x();

    let mut ppx = resolution_tile.precinct_exponent_x();
    let mut ppy = resolution_tile.precinct_exponent_y();

    let mut y_start = (resolution_tile.rect.y0 / (1 << ppy)) * (1 << ppy);
    let mut x_start = (resolution_tile.rect.x0 / (1 << ppx)) * (1 << ppx);

    // TODO: I don't really understand where the specification mentions this
    // is necessary. I just copied this from the Serenity decoder.
    if resolution_tile.resolution > 0 {
        ppx -= 1;
        ppy -= 1;

        x_start /= 2;
        y_start /= 2;
    }

    let ppx_pow2 = 1 << ppx;
    let ppy_pow2 = 1 << ppy;

    let start = storage.precincts.len();

    let mut y0 = y_start;
    for _y in 0..num_precincts_y {
        let mut x0 = x_start;

        for _x in 0..num_precincts_x {
            let precinct_rect = IntRect::from_xywh(x0, y0, ppx_pow2, ppy_pow2);

            let cb_width = resolution_tile.code_block_width();
            let cb_height = resolution_tile.code_block_height();

            let cb_x0 = (u32::max(precinct_rect.x0, sub_band_rect.x0) / cb_width) * cb_width;
            let cb_y0 = (u32::max(precinct_rect.y0, sub_band_rect.y0) / cb_height) * cb_height;

            let code_block_area = IntRect::from_ltrb(
                cb_x0,
                cb_y0,
                u32::min(precinct_rect.x1, sub_band_rect.x1),
                u32::min(precinct_rect.y1, sub_band_rect.y1),
            );

            let code_blocks_x = if sub_band_rect.width() == 0 {
                0
            } else {
                code_block_area.width().div_ceil(cb_width)
            };
            let code_blocks_y = if sub_band_rect.height() == 0 {
                0
            } else {
                code_block_area.height().div_ceil(cb_height)
            };

            trace!(
                "Precinct rect: [{},{} {}x{}], num_code_blocks_wide: {}, num_code_blocks_high: {}",
                precinct_rect.x0,
                precinct_rect.y0,
                precinct_rect.width(),
                precinct_rect.height(),
                code_blocks_x,
                code_blocks_y
            );

            let blocks = build_code_blocks(
                code_block_area,
                sub_band_rect,
                resolution_tile,
                code_blocks_x,
                code_blocks_y,
                tile_ctx,
                storage,
            );

            let code_inclusion_tree = TagTree::new(code_blocks_x, code_blocks_y);
            let zero_bitplane_tree = TagTree::new(code_blocks_x, code_blocks_y);

            storage.precincts.push(Precinct {
                code_blocks: blocks,
                code_inclusion_tree,
                zero_bitplane_tree,
            });

            x0 += ppx_pow2;
        }

        y0 += ppy_pow2;
    }

    let end = storage.precincts.len();

    Ok(start..end)
}

fn build_code_blocks(
    code_block_area: IntRect,
    sub_band_rect: IntRect,
    tile_instance: &ResolutionTile,
    code_blocks_x: u32,
    code_blocks_y: u32,
    tile_ctx: &mut TileDecodeContext,
    storage: &mut DecompositionStorage,
) -> Range<usize> {
    let mut y = code_block_area.y0;

    let code_block_width = tile_instance.code_block_width();
    let code_block_height = tile_instance.code_block_height();

    let start = storage.code_blocks.len();

    for y_idx in 0..code_blocks_y {
        let mut x = code_block_area.x0;

        for x_idx in 0..code_blocks_x {
            let area = IntRect::from_xywh(x, y, code_block_width, code_block_height)
                .intersect(sub_band_rect);

            trace!(
                "Codeblock rect: [{},{} {}x{}]",
                area.x0,
                area.y0,
                area.width(),
                area.height(),
            );

            let start = storage.layers.len();
            storage.layers.extend(iter::repeat_n(
                Layer {
                    // This will be updated once we actually read the
                    // layer segments.
                    segments: None,
                },
                tile_ctx.tile.num_layers as usize,
            ));
            let end = storage.layers.len();

            storage.code_blocks.push(CodeBlock {
                x_idx,
                y_idx,
                rect: area,
                has_been_included: false,
                missing_bit_planes: 0,
                l_block: 3,
                number_of_coding_passes: 0,
                layers: start..end,
                non_empty_layer_count: 0,
            });

            x += code_block_width;
        }

        y += code_block_height;
    }

    let end = storage.code_blocks.len();

    start..end
}

fn get_code_block_data<'a>(
    tile: &'a Tile<'a>,
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage<'a>,
) -> Result<(), &'static str> {
    for tile_part in &tile.tile_parts {
        if get_code_block_data_inner(tile_part, &mut progression_iterator, tile_ctx, storage)
            .is_none()
        {
            warn!(
                "failed to fully process a tile part in tile {}, decoded image might be corrupted",
                tile.idx
            );
        }
    }

    Ok(())
}

fn get_code_block_data_inner<'a>(
    tile_part_data: &'a [u8],
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage<'a>,
) -> Option<()> {
    let mut data = tile_part_data;

    while !data.is_empty() {
        let progression_data = progression_iterator.next()?;
        let resolution = progression_data.resolution;
        let component_info = &tile_ctx.tile.component_infos[progression_data.component as usize];
        let tile_decompositions =
            &mut storage.tile_decompositions[progression_data.component as usize];
        let sub_band_iter = tile_decompositions.sub_band_iter(resolution, &storage.decompositions);

        if component_info.coding_style.flags.may_use_sop_markers() {
            let mut reader = Reader::new(data);
            if reader.peek_marker() == Some(SOP) {
                reader.read_marker().ok()?;
                reader.skip_bytes(4)?;
                data = reader.tail()?;
            }
        }

        let mut reader = BitReader::new(data);
        let zero_length = reader.read_bits_with_stuffing(1)? == 0;

        // B.10.3 Zero length packet
        // "The first bit in the packet header denotes whether the packet has a length of zero
        // (empty packet). The value 0 indicates a zero length; no code-blocks are included in this
        // case. The value 1 indicates a non-zero length."
        if !zero_length {
            for sub_band in sub_band_iter.clone() {
                get_code_block_lengths(
                    sub_band,
                    &progression_data,
                    &mut reader,
                    storage,
                    component_info,
                )?;
            }
        }

        // TODO: What to do with the note below B.10.3?
        // TODO: Support multiple codeword segments (10.7.2)

        reader.read_stuff_bit_if_necessary()?;
        reader.align();
        let packet_data = reader.tail();

        let mut data_reader = Reader::new(packet_data);

        if component_info.coding_style.flags.uses_eph_marker()
            && data_reader.read_marker().ok()? != EPH
        {
            return None;
        }

        if !zero_length {
            for sub_band in sub_band_iter {
                let sub_band = &mut storage.sub_bands[sub_band];
                let precinct = &mut storage.precincts[sub_band.precincts.clone()]
                    [progression_data.precinct as usize];
                let code_blocks = &mut storage.code_blocks[precinct.code_blocks.clone()];

                for code_block in code_blocks {
                    let layer = &mut storage.layers[code_block.layers.clone()]
                        [progression_data.layer_num as usize];

                    if let Some(segments) = layer.segments.clone() {
                        let segments = &mut storage.segments[segments.clone()];

                        for segment in segments {
                            segment.data = data_reader.read_bytes(segment.data_length as usize)?
                        }
                    }
                }
            }
        }

        data = data_reader.tail()?;
    }

    Some(())
}

fn get_code_block_lengths(
    sub_band_dx: usize,
    progression_data: &ProgressionData,
    reader: &mut BitReader,
    storage: &mut DecompositionStorage,
    component_info: &ComponentInfo,
) -> Option<()> {
    let precincts = &mut storage.precincts[storage.sub_bands[sub_band_dx].precincts.clone()];
    let precinct = &mut precincts[progression_data.precinct as usize];
    let code_blocks = &mut storage.code_blocks[precinct.code_blocks.clone()];

    for code_block in code_blocks {
        // B.10.4 Code-block inclusion
        let is_included = if code_block.has_been_included {
            // "For code-blocks that have been included in a previous packet,
            // a single bit is used to represent the information, where a 1
            // means that the code-block is included in this layer and a 0 means
            // that it is not."
            reader.read_bits_with_stuffing(1)? == 1
        } else {
            // "For code-blocks that have not been previously included in any packet,
            // this information is signalled with a separate tag tree code for each precinct
            // as confined to a sub-band. The values in this tag tree are the number of the
            // layer in which the current code-block is first included. Although the exact
            // sequence of bits that represent the inclusion tag tree appears in the bit
            // stream, only the bits needed for determining whether the code-block is
            // included are placed in the packet header. If some of the tag tree is already
            // known from previous code-blocks or previous layers, it is not repeated.
            // Likewise, only as much of the tag tree as is needed to determine inclusion in
            // the current layer is included. If a code-block is not included until a later
            // layer, then only a partial tag tree is included at that point in the bit
            // stream."
            precinct.code_inclusion_tree.read(
                code_block.x_idx,
                code_block.y_idx,
                reader,
                progression_data.layer_num as u32 + 1,
            )? <= progression_data.layer_num as u32
        };

        trace!("code-block inclusion: {}", is_included);

        if !is_included {
            continue;
        }

        let layer =
            &mut storage.layers[code_block.layers.clone()][progression_data.layer_num as usize];

        let included_first_time = is_included && !code_block.has_been_included;

        // B.10.5 Zero bit-plane information
        // "If a code-block is included for the first time, the packet header contains
        // information identifying the actual number of bit-planes used to represent
        // coefficients from the code-block. The maximum number of bit-planes available
        // for the representation of coefficients in any sub-band, b, is given by Mb as
        // defined in Equation (E-2). In general, however, the
        // number of actual bit-planes for which coding passes are generated is Mb – P,
        // where the number of missing most significant bit-planes, P, may vary from
        // code-block to code-block; these missing bit-planes are all taken to be zero. The
        // value of P is coded in the packet header with a separate tag tree for every
        // precinct, in the same manner as the code block inclusion information."
        if included_first_time {
            code_block.missing_bit_planes = precinct.zero_bitplane_tree.read(
                code_block.x_idx,
                code_block.y_idx,
                reader,
                u32::MAX,
            )? as u8;
            trace!(
                "zero bit-plane information: {}",
                code_block.missing_bit_planes
            );
        }

        code_block.has_been_included |= is_included;

        // B.10.6 Number of coding passes
        // "The number of coding passes included in this packet from each code-block is
        // identified in the packet header using the codewords shown in Table B.4. This
        // table provides for the possibility of signalling up to 164 coding passes.
        let added_coding_passes = if reader.peak_bits_with_stuffing(9) == Some(0x1ff) {
            reader.read_bits_with_stuffing(9)?;
            reader.read_bits_with_stuffing(7)? + 37
        } else if reader.peak_bits_with_stuffing(4) == Some(0x0f) {
            reader.read_bits_with_stuffing(4)?;
            // TODO: Validate that sequence is not 1111 1
            reader.read_bits_with_stuffing(5)? + 6
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1110) {
            reader.read_bits_with_stuffing(4)?;
            5
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1101) {
            reader.read_bits_with_stuffing(4)?;
            4
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1100) {
            reader.read_bits_with_stuffing(4)?;
            3
        } else if reader.peak_bits_with_stuffing(2) == Some(0b10) {
            reader.read_bits_with_stuffing(2)?;
            2
        } else if reader.peak_bits_with_stuffing(1) == Some(0) {
            reader.read_bits_with_stuffing(1)?;
            1
        } else {
            return None;
        };

        trace!("number of coding passes: {}", added_coding_passes);

        let mut k = 0;

        while reader.read_bits_with_stuffing(1)? == 1 {
            k += 1;
        }

        code_block.l_block += k;

        let previous_layers_passes = code_block.number_of_coding_passes;
        let cumulative_passes = previous_layers_passes + added_coding_passes;

        let get_segment = |code_block_idx: u32| {
            if component_info.code_block_style().termination_on_each_pass {
                code_block_idx
            } else if component_info
                .code_block_style()
                .selective_arithmetic_coding_bypass
            {
                segment_idx_for_bypass(code_block_idx)
            } else {
                code_block.non_empty_layer_count
            }
        };

        let start = storage.segments.len();

        let mut push_segment = |segment: u32, coding_passes_for_segment: u32| {
            let length = {
                assert!(coding_passes_for_segment > 0);

                // "A codeword segment is the number of bytes contributed to a packet by a
                // code-block. The length of a codeword segment is represented by a binary number of length:
                // bits = Lblock + floor(log_2(coding passes added))
                // where Lblock is a code-block state variable. A separate Lblock is used for each
                // code-block in the precinct. The value of Lblock is initially set to three. The
                // number of bytes contributed by each code-block is preceded by signalling bits
                // that increase the value of Lblock, as needed. A signalling bit of zero indicates
                // the current value of Lblock is sufficient. If there are k ones followed by a
                // zero, the value of Lblock is incremented by k. While Lblock can only increase,
                // the number of bits used to signal the length of the code-block contribution can
                // increase or decrease depending on the number of coding passes included."
                let length_bits = code_block.l_block + coding_passes_for_segment.ilog2();
                reader.read_bits_with_stuffing(length_bits as u8)
            }?;

            storage.segments.push(Segment {
                idx: segment,
                data_length: length,
                coding_pases: coding_passes_for_segment,
                // Will be set later.
                data: &[],
            });

            trace!("length({segment}) {}", length);

            Some(())
        };

        let mut last_segment = get_segment(previous_layers_passes);
        let mut coding_passes_for_segment = 0;

        for coding_pass in previous_layers_passes..cumulative_passes {
            let segment = get_segment(coding_pass);

            if segment != last_segment {
                push_segment(last_segment, coding_passes_for_segment)?;
                last_segment = segment;
                coding_passes_for_segment = 1;
            } else {
                coding_passes_for_segment += 1;
            }
        }

        // Flush the final segment if applicable.
        if coding_passes_for_segment > 0 {
            push_segment(last_segment, coding_passes_for_segment)?;
        }

        let end = storage.segments.len();
        layer.segments = Some(start..end);
        code_block.number_of_coding_passes += added_coding_passes;
        code_block.non_empty_layer_count += 1;
    }

    Some(())
}

fn segment_idx_for_bypass(code_block_idx: u32) -> u32 {
    if code_block_idx < 10 {
        0
    } else {
        1 + (2 * ((code_block_idx - 10) / 3))
            + (if ((code_block_idx - 10) % 3) == 2 {
                1
            } else {
                0
            })
    }
}

fn decode_bitplanes<'a>(
    tile: &'a Tile<'a>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage,
) -> Result<(), &'static str> {
    for (tile_decompositions_idx, component_info) in tile.component_infos.iter().enumerate() {
        for resolution in 0..component_info.num_resolution_levels() {
            let tile_composition = &storage.tile_decompositions[tile_decompositions_idx];
            let sub_band_iter = tile_composition.sub_band_iter(resolution, &storage.decompositions);

            for sub_band_idx in sub_band_iter {
                decode_sub_band_bitplanes(
                    sub_band_idx,
                    resolution,
                    component_info,
                    &mut tile_ctx.code_block_decode_context,
                    storage,
                )?;
            }
        }
    }

    Ok(())
}

fn decode_sub_band_bitplanes(
    sub_band_idx: usize,
    resolution: u16,
    component_info: &ComponentInfo,
    b_ctx: &mut CodeBlockDecodeContext,
    storage: &mut DecompositionStorage,
) -> Result<(), &'static str> {
    let sub_band = &mut storage.sub_bands[sub_band_idx];

    let dequantization_step = {
        if component_info.quantization_info.quantization_style == QuantizationStyle::NoQuantization
        {
            None
        } else {
            let (exponent, mantissa) =
                component_info.exponent_mantissa(sub_band.sub_band_type, resolution);

            let r_b = {
                let log_gain = match sub_band.sub_band_type {
                    SubBandType::LowLow => 0,
                    SubBandType::LowHigh => 1,
                    SubBandType::HighLow => 1,
                    SubBandType::HighHigh => 2,
                };

                component_info.size_info.precision as u16 + log_gain
            };
            let delta_b = 2.0f32.powf(r_b as f32 - exponent as f32)
                * (1.0 + (mantissa as f32) / (2u32.pow(11) as f32));

            Some(delta_b)
        }
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
            let num_bitplanes = {
                let (exponent, _) =
                    component_info.exponent_mantissa(sub_band.sub_band_type, resolution);
                // Equation (E-2)
                component_info.quantization_info.guard_bits as u16 + exponent - 1
            };

            bitplane::decode(
                code_block,
                sub_band.sub_band_type,
                num_bitplanes,
                &component_info.coding_style.parameters.code_block_style,
                b_ctx,
                &storage.layers[code_block.layers.start..code_block.layers.end],
                &storage.segments,
            )?;

            // Turn the signs and magnitudes into singular coefficients and
            // copy them into the sub-band.

            let x_offset = code_block.rect.x0 - sub_band.rect.x0;
            let y_offset = code_block.rect.y0 - sub_band.rect.y0;

            let sign_iter = b_ctx.signs().chunks_exact(code_block.rect.width() as usize);
            let magnitude_iter = b_ctx
                .magnitudes()
                .chunks_exact(code_block.rect.width() as usize);

            for (y, (signs, magnitudes)) in sign_iter.zip(magnitude_iter).enumerate() {
                let out_row = &mut sub_band.coefficients[((y_offset + y as u32)
                    * sub_band.rect.width())
                    as usize
                    + x_offset as usize..];

                for ((output, sign), magnitude) in out_row
                    .iter_mut()
                    .zip(signs.iter().copied())
                    .zip(magnitudes.iter().copied())
                {
                    *output = magnitude.get() as f32;

                    if sign != 0 {
                        *output = -*output;
                    }

                    if let Some(q) = dequantization_step {
                        *output *= q;
                    }
                }
            }
        }
    }

    Ok(())
}

fn apply_idwt<'a>(
    tile: &'a Tile<'a>,
    tile_ctx: &mut TileDecodeContext<'a>,
    storage: &mut DecompositionStorage,
) -> Result<(), &'static str> {
    for (decompositions, component_info) in storage
        .tile_decompositions
        .iter()
        .zip(tile.component_infos.iter())
    {
        let ll_sub_band = &storage.sub_bands[decompositions.first_ll_sub_band];
        let sub_bands = &storage.decompositions[decompositions.decompositions.clone()];
        let idwt_output = idwt::apply(
            ll_sub_band,
            sub_bands,
            &storage.sub_bands,
            component_info.coding_style.parameters.transformation,
        );

        tile_ctx.idwt_outputs.push(idwt_output);
    }

    Ok(())
}

fn apply_mct(tile_ctx: &mut TileDecodeContext) {
    if tile_ctx.tile.mct {
        if tile_ctx.idwt_outputs.len() < 3 {
            warn!(
                "tried to apply MCT to image with {} components",
                tile_ctx.idwt_outputs.len()
            );

            return;
        }

        let (s, _) = tile_ctx.idwt_outputs.split_at_mut(3);
        let [s0, s1, s2] = s else { unreachable!() };
        let s0 = &mut s0.coefficients;
        let s1 = &mut s1.coefficients;
        let s2 = &mut s2.coefficients;

        let transform = tile_ctx.tile.component_infos[0].wavelet_transform();

        if transform != tile_ctx.tile.component_infos[1].wavelet_transform()
            || tile_ctx.tile.component_infos[1].wavelet_transform()
                != tile_ctx.tile.component_infos[2].wavelet_transform()
        {
            warn!("tried to apply MCT to image with different wavelet transforms per component");
            return;
        }

        let len = s0.len();

        if len != s1.len() || s1.len() != s2.len() {
            warn!("tried to apply MCT to image with different number of samples per component");
            return;
        }

        match transform {
            WaveletTransform::Irreversible97 => {
                for ((y0, y1), y2) in s0.iter_mut().zip(s1.iter_mut()).zip(s2.iter_mut()) {
                    let i0 = *y0 + 1.402 * *y2;
                    let i1 = *y0 - 0.34413 * *y1 - 0.71414 * *y2;
                    let i2 = *y0 + 1.772 * *y1;

                    *y0 = i0;
                    *y1 = i1;
                    *y2 = i2;
                }
            }
            WaveletTransform::Reversible53 => {
                for ((y0, y1), y2) in s0.iter_mut().zip(s1.iter_mut()).zip(s2.iter_mut()) {
                    let i1 = *y0 - ((*y2 + *y1) / 4.0).floor();
                    let i0 = *y2 + i1;
                    let i2 = *y1 + i1;

                    *y0 = i0;
                    *y1 = i1;
                    *y2 = i2;
                }
            }
        }
    }
}

fn store<'a>(tile: &'a Tile<'a>, header: &Header, tile_ctx: &mut TileDecodeContext<'a>) {
    let width = header.size_data.tile_width;
    let height = header.size_data.tile_height;

    for ((idwt_output, component_info), channel_data) in tile_ctx
        .idwt_outputs
        .iter_mut()
        .zip(tile.component_infos.iter())
        .zip(tile_ctx.channel_data.iter_mut())
    {
        for sample in idwt_output.coefficients.iter_mut() {
            *sample += (1 << (component_info.size_info.precision - 1)) as f32;
        }

        let component_tile = ComponentTile::new(tile, component_info);

        assert_eq!(idwt_output.rect, component_tile.rect);

        let (scale_x, scale_y) = (
            component_info.size_info.horizontal_resolution,
            component_info.size_info.vertical_resolution,
        );

        if scale_x == 1 && scale_y == 1 {
            // If no sub-sampling, use a fast path where we copy rows of coefficients
            // at once.

            // The rect of the IDWT output corresponds to the rect of the highest
            // decomposition level of the tile, which is usually not 1:1 aligned
            // with the actual tile rectangle. We also need to account for the
            // offset of the reference grid.
            let (image_x_offset, image_y_offset) = (
                header.size_data.image_area_x_offset,
                header.size_data.image_area_y_offset,
            );

            let skip_x = image_x_offset.saturating_sub(idwt_output.rect.x0);
            let skip_y = image_y_offset.saturating_sub(idwt_output.rect.y0);

            let input_row_iter = idwt_output
                .coefficients
                .chunks_exact(idwt_output.rect.width() as usize)
                .skip(skip_y as usize);

            let output_row_iter = channel_data
                .container
                .chunks_exact_mut(header.size_data.image_width() as usize)
                .skip(tile.rect.y0.saturating_sub(image_y_offset) as usize);

            for (input_row, output_row) in input_row_iter.zip(output_row_iter) {
                let input_row = &input_row[skip_x as usize..];
                let output_row = &mut output_row
                    [tile.rect.x0.saturating_sub(image_x_offset) as usize..][..input_row.len()];

                output_row.copy_from_slice(input_row);
            }
        } else {
            // Currently, we can assume that the reference grid offset is 0
            // (we have a check for that when parsing size data) for simplicity.

            // Otherwise, copy sample by sample.
            for y in component_tile.rect.y0..component_tile.rect.y1 {
                let relative_y = (y - component_tile.rect.y0) as usize;
                let reference_grid_y = scale_y as u32 * y;

                for x in component_tile.rect.x0..component_tile.rect.x1 {
                    let relative_x = (x - component_tile.rect.x0) as usize;
                    let reference_grid_x = scale_x as u32 * x;

                    let sample = idwt_output.coefficients
                        [relative_y * component_tile.rect.width() as usize + relative_x];

                    for x_position in
                        reference_grid_x..u32::min(reference_grid_x + scale_x as u32, width)
                    {
                        for y_position in
                            reference_grid_y..u32::min(reference_grid_y + scale_y as u32, height)
                        {
                            let pos = y_position as usize * width as usize + x_position as usize;

                            channel_data.container[pos] = sample;
                        }
                    }
                }
            }
        }
    }
}

pub(crate) trait BitReaderExt {
    fn read_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32>;
    fn read_stuff_bit_if_necessary(&mut self) -> Option<()>;
    fn peak_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32>;
}

impl BitReaderExt for BitReader<'_> {
    // Like the normal `read_bits` method, but accounts for stuffing bits
    // in addition.
    fn read_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32> {
        let mut bit = 0;

        for _ in 0..bit_size {
            self.read_stuff_bit_if_necessary()?;
            bit = (bit << 1) | self.read(1)?;
        }

        Some(bit)
    }

    fn read_stuff_bit_if_necessary(&mut self) -> Option<()> {
        // B.10.1: "If the value of the byte is 0xFF, the next byte includes an extra zero bit
        // stuffed into the MSB.
        // Check if the next bit is at a new byte boundary."
        if self.bit_pos() == 0 && self.byte_pos() > 0 {
            let last_byte = self.data[self.byte_pos() - 1];

            if last_byte == 0xff {
                let stuff_bit = self.read(1)?;

                if stuff_bit != 0 {
                    return None;
                }
            }
        }

        Some(())
    }

    fn peak_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32> {
        self.clone().read_bits_with_stuffing(bit_size)
    }
}
