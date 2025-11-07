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
use crate::tile::{ResolutionTile, Tile};
use crate::{bitplane, idwt, tile};
use hayro_common::bit::BitReader;
use hayro_common::byte::Reader;
use log::{trace, warn};

pub(crate) fn decode(data: &[u8], header: &Header) -> Result<Vec<ChannelData>, &'static str> {
    let mut reader = Reader::new(data);
    let tiles = tile::parse(&mut reader, header)?;

    let mut tile_ctx = TileDecodeContext::new(header);

    for (tile_idx, tile) in tiles.iter().enumerate() {
        trace!(
            "tile {tile_idx} rect [{},{} {}x{}]",
            tile.rect.x0,
            tile.rect.y0,
            tile.rect.width(),
            tile.rect.height(),
        );

        let iter_input = IteratorInput::new(
            tile,
            &header.component_infos,
            header.global_coding_style.num_layers,
        );

        match header.global_coding_style.progression_order {
            ProgressionOrder::LayerResolutionComponentPosition => {
                let iterator = build_layer_resolution_component_position_sequence(&iter_input);
                decode_tile(tile, header, iterator.into_iter(), &mut tile_ctx)?
            }
            ProgressionOrder::ResolutionLayerComponentPosition => {
                let iterator = build_resolution_layer_component_position_sequence(&iter_input);
                decode_tile(tile, header, iterator.into_iter(), &mut tile_ctx)?
            }
            ProgressionOrder::ResolutionPositionComponentLayer => {
                let iterator = build_resolution_position_component_layer_sequence(&iter_input);
                decode_tile(tile, header, iterator.into_iter(), &mut tile_ctx)?
            }
            ProgressionOrder::PositionComponentResolutionLayer => {
                let iterator = build_position_component_resolution_layer_sequence(&iter_input);
                decode_tile(tile, header, iterator.into_iter(), &mut tile_ctx)?
            }
            ProgressionOrder::ComponentPositionResolutionLayer => {
                let iterator = build_component_position_resolution_layer_sequence(&iter_input);
                decode_tile(tile, header, iterator.into_iter(), &mut tile_ctx)?
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
) -> Result<(), &'static str> {
    tile_ctx.reset();

    // This is the method that orchestrates all steps.

    // First, we build the decompositions, including their sub-bands, precincts
    // and code blocks.
    build_tile_decompositions(tile, header, tile_ctx)?;
    // Next, we parse the layer data for each code block.
    get_code_block_data(tile, header, progression_iterator, tile_ctx)?;
    // We then decode the bitplanes of each code block, yielding the
    // (possibly dequantized) coefficients of each code block.
    decode_bitplanes(tile, tile_ctx)?;
    // Next, we apply the inverse discrete wavelet transform.
    apply_idwt(tile, tile_ctx)?;
    // If applicable, we apply the multi-component transform.
    apply_mct(header, tile_ctx);
    // Finally, we store the raw samples for the tile area in the correct
    // location.
    store(tile, header, tile_ctx);

    Ok(())
}

/// All decompositions for a single tile.
struct TileDecompositions<'a> {
    first_ll_sub_band: SubBand<'a>,
    decompositions: Vec<Decomposition<'a>>,
}

impl<'a> TileDecompositions<'a> {
    fn for_each_sub_band<T>(
        &mut self,
        resolution: u16,
        mut func: impl FnMut(&mut SubBand<'a>) -> Option<T>,
    ) -> Option<()> {
        if resolution == 0 {
            func(&mut self.first_ll_sub_band)?;
        } else {
            let decomposition = &mut self.decompositions[resolution as usize - 1];

            for sub_band in &mut decomposition.sub_bands {
                func(sub_band)?;
            }
        }

        Some(())
    }
}

pub(crate) struct Decomposition<'a> {
    /// In the order low-high, high-low and high-high.
    pub(crate) sub_bands: [SubBand<'a>; 3],
    /// The rectangle of the decomposition.
    pub(crate) rect: IntRect,
}

#[derive(Clone)]
pub(crate) struct SubBand<'a> {
    pub(crate) sub_band_type: SubBandType,
    pub(crate) rect: IntRect,
    pub(crate) precincts: Vec<Precinct<'a>>,
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
pub(crate) struct Precinct<'a> {
    code_blocks: Vec<CodeBlock<'a>>,
    code_inclusion_tree: TagTree,
    zero_bitplane_tree: TagTree,
}

#[derive(Clone)]
pub(crate) struct CodeBlock<'a> {
    pub(crate) rect: IntRect,
    pub(crate) x_idx: u32,
    pub(crate) y_idx: u32,
    pub(crate) layer_data: Vec<&'a [u8]>,
    pub(crate) has_been_included: bool,
    pub(crate) missing_bit_planes: u8,
    pub(crate) number_of_coding_passes: u32,
    pub(crate) l_block: u32,
}

/// A reusable context used during the decoding of a single tile.
///
/// Some of the fields are temporary in nature and reset after moving on to the
/// next tile, some contain global state.
struct TileDecodeContext<'a> {
    /// The decompositions of each component of the tile we are currently
    /// processing.
    decompositions: Vec<TileDecompositions<'a>>,
    /// The outputs of the IDWT operations of each component of the tile
    /// we are currently processing.
    idwt_outputs: Vec<IDWTOutput>,
    /// A reusable context for decoding code blocks.
    code_block_decode_context: CodeBlockDecodeContext,
    /// A reusable temporary buffer used to store the lengths of codeblocks.
    code_block_len_buf: Vec<u32>,
    /// The raw, decoded samples for each channel.
    channel_data: Vec<ChannelData>,
}

impl TileDecodeContext<'_> {
    fn new(header: &Header) -> Self {
        let mut channel_data = vec![];

        for info in &header.component_infos {
            channel_data.push(ChannelData {
                container: vec![
                    0.0;
                    (header.size_data.reference_grid_width * header.size_data.reference_grid_height)
                        as usize
                ],
                // Will be set later on, because that data only exists in the
                // metadata of the JP2 file, not the actual code stream.
                is_alpha: false,
                bit_depth: info.size_info.precision,
            })
        }

        Self {
            decompositions: vec![],
            idwt_outputs: vec![],
            code_block_decode_context: Default::default(),
            code_block_len_buf: vec![],
            channel_data,
        }
    }

    fn reset(&mut self) {
        self.code_block_len_buf.clear();
        self.decompositions.clear();
        self.idwt_outputs.clear();
        // Code-block decode context will be resetted before being used.
        // Channel data should not be resetted because it's global
    }
}

fn build_tile_decompositions(
    tile: &Tile,
    header: &Header,
    tile_ctx: &mut TileDecodeContext,
) -> Result<(), &'static str> {
    for (component_idx, component_tile) in tile.component_tiles().enumerate() {
        // TODO: IMprove this
        let mut ll_sub_band = None;
        let mut decompositions = vec![];

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
                let precincts = build_precincts(&resolution_tile, sub_band_rect, header)?;

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
                let build_sub_band = |sub_band_type: SubBandType| {
                    let sub_band_rect = resolution_tile.sub_band_rect(sub_band_type);

                    let precincts = build_precincts(&resolution_tile, sub_band_rect, header)?;

                    Ok(SubBand {
                        sub_band_type,
                        rect: sub_band_rect,
                        precincts: precincts.clone(),
                        coefficients: vec![
                            0.0;
                            (sub_band_rect.width() * sub_band_rect.height()) as usize
                        ],
                    })
                };

                let decomposition = Decomposition {
                    sub_bands: [
                        build_sub_band(SubBandType::HighLow)?,
                        build_sub_band(SubBandType::LowHigh)?,
                        build_sub_band(SubBandType::HighHigh)?,
                    ],
                    rect: resolution_tile.rect,
                };

                decompositions.push(decomposition);
            }
        }

        tile_ctx.decompositions.push(TileDecompositions {
            decompositions,
            first_ll_sub_band: ll_sub_band.unwrap(),
        });
    }

    Ok(())
}

fn build_precincts(
    resolution_tile: &ResolutionTile,
    sub_band_rect: IntRect,
    header: &Header,
) -> Result<Vec<Precinct<'static>>, &'static str> {
    let mut precincts = vec![];

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
                header.global_coding_style.num_layers,
            );

            let code_inclusion_tree = TagTree::new(code_blocks_x, code_blocks_y);
            let zero_bitplane_tree = TagTree::new(code_blocks_x, code_blocks_y);

            precincts.push(Precinct {
                code_blocks: blocks,
                code_inclusion_tree,
                zero_bitplane_tree,
            });

            x0 += ppx_pow2;
        }

        y0 += ppy_pow2;
    }

    Ok(precincts)
}

fn build_code_blocks(
    code_block_area: IntRect,
    sub_band_rect: IntRect,
    tile_instance: &ResolutionTile,
    code_blocks_x: u32,
    code_blocks_y: u32,
    num_layers: u16,
) -> Vec<CodeBlock<'static>> {
    let mut blocks = vec![];

    let mut y = code_block_area.y0;

    let code_block_width = tile_instance.code_block_width();
    let code_block_height = tile_instance.code_block_height();

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

            blocks.push(CodeBlock {
                x_idx,
                y_idx,
                rect: area,
                layer_data: vec![&[]; num_layers as usize],
                has_been_included: false,
                missing_bit_planes: 0,
                l_block: 3,
                number_of_coding_passes: 0,
            });

            x += code_block_width;
        }

        y += code_block_height;
    }

    blocks
}

fn get_code_block_data<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
    tile_ctx: &mut TileDecodeContext<'a>,
) -> Result<(), &'static str> {
    for tile_part in &tile.tile_parts {
        get_code_block_data_inner(*tile_part, header, &mut progression_iterator, tile_ctx)
            .ok_or("failed to parse packet for tile")?;
    }

    Ok(())
}

fn get_code_block_data_inner<'a>(
    tile_part_data: &'a [u8],
    header: &Header,
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
    tile_ctx: &mut TileDecodeContext<'a>,
) -> Option<()> {
    let mut data = tile_part_data;

    while !data.is_empty() {
        tile_ctx.code_block_len_buf.clear();

        if header
            .global_coding_style
            .component_parameters
            .flags
            .may_use_sop_markers()
        {
            let mut reader = Reader::new(data);
            if reader.peek_marker() == Some(SOP) {
                reader.read_marker().ok()?;
                reader.skip_bytes(4)?;
                data = reader.tail()?;
            }
        }

        let mut reader = BitReader::new(data);

        let progression_data = progression_iterator.next()?;
        let resolution = progression_data.resolution;
        let zero_length = reader.read_packet_header_bits(1)? == 0;

        let component_data = &mut tile_ctx.decompositions[progression_data.component as usize];

        // B.10.3 Zero length packet
        // "The first bit in the packet header denotes whether the packet has a length of zero
        // (empty packet). The value 0 indicates a zero length; no code-blocks are included in this
        // case. The value 1 indicates a non-zero length."
        if !zero_length {
            component_data.for_each_sub_band(resolution, |sub_band| {
                get_code_block_lengths(
                    sub_band,
                    &progression_data,
                    &mut reader,
                    &mut tile_ctx.code_block_len_buf,
                )
            })?;
        }

        // TODO: What to do with the note below B.10.3?
        // TODO: Support multiple codeword segments (10.7.2)

        reader.read_stuff_bit_if_necessary()?;
        reader.align();
        let packet_data = reader.tail();

        let mut data_reader = Reader::new(packet_data);

        if header
            .global_coding_style
            .component_parameters
            .flags
            .uses_eph_marker()
            && data_reader.read_marker().ok()? != EPH
        {
            return None;
        }

        if !zero_length {
            let mut entries = tile_ctx.code_block_len_buf.iter().copied();

            component_data.for_each_sub_band(resolution, |sub_band| {
                let precinct = &mut sub_band.precincts[progression_data.precinct as usize];

                for code_block in &mut precinct.code_blocks {
                    let length = entries.next()?;

                    let layer = &mut code_block.layer_data[progression_data.layer_num as usize];
                    *layer = data_reader.read_bytes(length as usize)?;
                }
                Some(())
            })?;
        }

        data = data_reader.tail()?;
    }

    Some(())
}

fn get_code_block_lengths(
    sub_band: &mut SubBand,
    progression_data: &ProgressionData,
    reader: &mut BitReader,
    code_block_lengths: &mut Vec<u32>,
) -> Option<()> {
    let precinct = &mut sub_band.precincts[progression_data.precinct as usize];

    for code_block in &mut precinct.code_blocks {
        // B.10.4 Code-block inclusion
        let is_included = if code_block.has_been_included {
            // "For code-blocks that have been included in a previous packet,
            // a single bit is used to represent the information, where a 1
            // means that the code-block is included in this layer and a 0 means
            // that it is not."
            reader.read_packet_header_bits(1)? == 1
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
            code_block_lengths.push(0);
            continue;
        }

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
        let added_coding_passes = if reader.peak_packet_header_bits(9) == Some(0x1ff) {
            reader.read_packet_header_bits(9)?;
            reader.read_packet_header_bits(7)? + 37
        } else if reader.peak_packet_header_bits(4) == Some(0x0f) {
            reader.read_packet_header_bits(4)?;
            // TODO: Validate that sequence is not 1111 1
            reader.read_packet_header_bits(5)? + 6
        } else if reader.peak_packet_header_bits(4) == Some(0b1110) {
            reader.read_packet_header_bits(4)?;
            5
        } else if reader.peak_packet_header_bits(4) == Some(0b1101) {
            reader.read_packet_header_bits(4)?;
            4
        } else if reader.peak_packet_header_bits(4) == Some(0b1100) {
            reader.read_packet_header_bits(4)?;
            3
        } else if reader.peak_packet_header_bits(2) == Some(0b10) {
            reader.read_packet_header_bits(2)?;
            2
        } else if reader.peak_packet_header_bits(1) == Some(0) {
            reader.read_packet_header_bits(1)?;
            1
        } else {
            return None;
        };

        code_block.number_of_coding_passes += added_coding_passes;

        trace!("number of coding passes: {}", added_coding_passes);

        // B.10.7.1 Single codeword segment
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
        let mut k = 0;

        while reader.read_packet_header_bits(1)? == 1 {
            k += 1;
        }

        code_block.l_block += k;
        let length_bits = code_block.l_block + added_coding_passes.ilog2();
        let length = reader.read_packet_header_bits(length_bits as u8)?;
        code_block_lengths.push(length);

        trace!("length(0) {}", length);
    }

    Some(())
}

fn decode_bitplanes<'a>(
    tile: &'a Tile<'a>,
    tile_ctx: &mut TileDecodeContext<'a>,
) -> Result<(), &'static str> {
    for (decompositions, component_info) in tile_ctx
        .decompositions
        .iter_mut()
        .zip(tile.component_infos.iter())
    {
        decode_sub_band_bitplanes(
            &mut decompositions.first_ll_sub_band,
            0,
            component_info,
            &mut tile_ctx.code_block_decode_context,
        )?;

        for (resolution, decomposition) in decompositions.decompositions.iter_mut().enumerate() {
            for sub_band in &mut decomposition.sub_bands {
                decode_sub_band_bitplanes(
                    sub_band,
                    resolution as u16 + 1,
                    component_info,
                    &mut tile_ctx.code_block_decode_context,
                )?;
            }
        }
    }

    Ok(())
}

fn decode_sub_band_bitplanes(
    sub_band: &mut SubBand,
    resolution: u16,
    component_info: &ComponentInfo,
    b_ctx: &mut CodeBlockDecodeContext,
) -> Result<(), &'static str> {
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

    for precinct in &mut sub_band.precincts {
        for codeblock in &mut precinct.code_blocks {
            let num_bitplanes = {
                let (exponent, _) =
                    component_info.exponent_mantissa(sub_band.sub_band_type, resolution);
                // Equation (E-2)
                component_info.quantization_info.guard_bits as u16 + exponent - 1
            };

            bitplane::decode(
                codeblock,
                sub_band.sub_band_type,
                num_bitplanes,
                &component_info.coding_style.parameters.code_block_style,
                b_ctx,
            )?;

            // Turn the signs and magnitudes into singular coefficients and
            // copy them into the sub-band.

            let x_offset = codeblock.rect.x0 - sub_band.rect.x0;
            let y_offset = codeblock.rect.y0 - sub_band.rect.y0;

            let sign_iter = b_ctx.signs().chunks_exact(codeblock.rect.width() as usize);
            let magnitude_iter = b_ctx
                .magnitudes()
                .chunks_exact(codeblock.rect.width() as usize);

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
) -> Result<(), &'static str> {
    for (decompositions, component_info) in tile_ctx
        .decompositions
        .iter_mut()
        .zip(tile.component_infos.iter())
    {
        let idwt_output = idwt::apply(
            &decompositions.first_ll_sub_band,
            &decompositions.decompositions,
            component_info.coding_style.parameters.transformation,
        );

        tile_ctx.idwt_outputs.push(idwt_output);
    }

    Ok(())
}

fn apply_mct<'a>(header: &Header, tile_ctx: &mut TileDecodeContext<'a>) {
    if header.global_coding_style.mct {
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

        let transform = header.component_infos[0].wavelet_transform();

        if transform != header.component_infos[1].wavelet_transform()
            || header.component_infos[1].wavelet_transform()
                != header.component_infos[2].wavelet_transform()
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
    for ((idwt_output, component_info), channel_data) in tile_ctx
        .idwt_outputs
        .iter_mut()
        .zip(header.component_infos.iter())
        .zip(tile_ctx.channel_data.iter_mut())
    {
        for sample in idwt_output.coefficients.iter_mut() {
            *sample += (1 << (component_info.size_info.precision - 1)) as f32;
        }

        // The rect of the IDWT output corresponds to the rect of the highest
        // decomposition level of the tile, which is usually not 1:1 aligned
        // with the actual tile rectangle. We also need to account for the
        // offset of the reference grid.

        let skip_x = tile.rect.x0 - idwt_output.rect.x0;
        let skip_y = tile.rect.y0 - idwt_output.rect.y0;
        let take_x = tile.rect.width();
        let take_y = tile.rect.height();

        let input_row_iter = idwt_output
            .coefficients
            .chunks_exact(idwt_output.rect.width() as usize)
            .skip(skip_y as usize)
            .take(take_y as usize);

        let output_row_iter = channel_data
            .container
            .chunks_exact_mut(header.size_data.reference_grid_width as usize)
            .skip(tile.rect.y0 as usize)
            .take(take_y as usize);

        for (input_row, output_row) in input_row_iter.zip(output_row_iter) {
            let input_row = &input_row[skip_x as usize..][..take_x as usize];
            let output_row = &mut output_row[tile.rect.x0 as usize..][..take_x as usize];

            output_row.copy_from_slice(input_row);
        }
    }
}

pub(crate) trait BitReaderExt {
    fn read_packet_header_bits(&mut self, bit_size: u8) -> Option<u32>;
    fn read_stuff_bit_if_necessary(&mut self) -> Option<()>;
    fn peak_packet_header_bits(&mut self, bit_size: u8) -> Option<u32>;
}

impl BitReaderExt for BitReader<'_> {
    // Like the normal `read_bits` method, but accounts for stuffing bits
    // in addition.
    fn read_packet_header_bits(&mut self, bit_size: u8) -> Option<u32> {
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

    fn peak_packet_header_bits(&mut self, bit_size: u8) -> Option<u32> {
        self.clone().read_packet_header_bits(bit_size)
    }
}
