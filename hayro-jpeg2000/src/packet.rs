use crate::bitmap::ChannelData;
use crate::bitplane::BitplaneDecodeContext;
use crate::codestream::markers::{EPH, SOP};
use crate::codestream::{
    ComponentInfo, Header, MultipleComponentTransform, ProgressionOrder, QuantizationStyle,
    ReaderExt, WaveletTransform,
};
use crate::idwt::IDWTOutput;
use crate::progression::{
    IteratorInput, ProgressionData, build_component_position_resolution_layer_sequence,
    build_layer_resolution_component_position_sequence,
    build_position_component_resolution_layer_sequence,
    build_resolution_layer_component_position_sequence,
    build_resolution_position_component_layer_sequence,
};
use crate::tag_tree::TagTree;
use crate::tile::{IntRect, Tile, TileInstance, TilePart};
use crate::{bitplane, idwt};
use hayro_common::bit::BitReader;
use hayro_common::byte::Reader;

pub(crate) struct Decomposition<'a> {
    /// In the order low-high, high-low and high-high.
    pub(crate) sub_bands: [SubBand<'a>; 3],
    pub(crate) rect: IntRect,
}

struct ComponentData<'a> {
    first_ll_sub_band: SubBand<'a>,
    decompositions: Vec<Decomposition<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubBandType {
    LowLow,
    LowHigh,
    HighLow,
    HighHigh,
}

#[derive(Clone)]
pub(crate) struct SubBand<'a> {
    pub(crate) sub_band_type: SubBandType,
    pub(crate) rect: IntRect,
    pub(crate) precincts: Vec<Precinct<'a>>,
    pub(crate) coefficients: Vec<f32>,
}

#[derive(Clone)]
pub(crate) struct Precinct<'a> {
    area: IntRect,
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
    pub(crate) coefficients: Vec<i16>,
}

#[derive(Clone)]
struct Segment {
    number_of_coding_passes: u32,
}

pub(crate) fn process_tiles(
    tiles: &[Tile],
    header: &Header,
) -> Result<Vec<ChannelData>, &'static str> {
    let mut channels = vec![];

    for (idx, info) in header.component_infos.iter().enumerate() {
        channels.push(ChannelData {
            container: vec![
                0.0;
                (header.size_data.reference_grid_width * header.size_data.reference_grid_height)
                    as usize
            ],
            // Will be set later on.
            is_alpha: false,
            bit_depth: info.size_info.precision,
        })
    }

    for (tile_idx, tile) in tiles.iter().enumerate() {
        // eprintln!(
        //     "tile {tile_idx} rect [{},{} {}x{}]",
        //     tile.rect.x0,
        //     tile.rect.y0,
        //     tile.rect.width(),
        //     tile.rect.height(),
        // );

        let iter_input = IteratorInput::new(
            tile,
            &header.component_infos,
            header.global_coding_style.num_layers,
        );

        let mut samples = match header.global_coding_style.progression_order {
            ProgressionOrder::LayerResolutionComponentPosition => {
                let iterator = build_layer_resolution_component_position_sequence(&iter_input);
                process_tile(tile, header, iterator.into_iter())?
            }
            ProgressionOrder::ResolutionLayerComponentPosition => {
                let iterator = build_resolution_layer_component_position_sequence(&iter_input);
                process_tile(tile, header, iterator.into_iter())?
            }
            ProgressionOrder::ResolutionPositionComponentLayer => {
                let iterator = build_resolution_position_component_layer_sequence(&iter_input);
                process_tile(tile, header, iterator.into_iter())?
            }
            ProgressionOrder::PositionComponentResolutionLayer => {
                let iterator = build_position_component_resolution_layer_sequence(&iter_input);
                process_tile(tile, header, iterator.into_iter())?
            }
            ProgressionOrder::ComponentPositionResolutionLayer => {
                let iterator = build_component_position_resolution_layer_sequence(&iter_input);
                process_tile(tile, header, iterator.into_iter())?
            }
        };

        save_samples(tile, header, &mut channels, &mut samples)
            .ok_or("failed to save decoded samples into channels")?;
    }

    Ok(channels)
}

fn process_tile<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
) -> Result<Vec<IDWTOutput>, &'static str> {
    let mut component_data = build_component_data(tile, header)?;

    for tile_part in tile.tile_parts() {
        parse_packet(
            &tile_part,
            header,
            &mut component_data,
            &mut progression_iterator,
        )
        .ok_or("failed to parse packet for tile")?;
    }

    let mut idw_output = vec![];

    // Create the context once and then reuse it so that we can reuse the
    // allocations.
    // TODO: Reuse across different tiles.
    let mut b_ctx = BitplaneDecodeContext::new();
    let mut layer_buffer = vec![];

    for (component_data, component_info) in
        component_data.iter_mut().zip(tile.component_info.iter())
    {
        process_sub_band(
            &mut component_data.first_ll_sub_band,
            0,
            component_info,
            &mut b_ctx,
            &mut layer_buffer,
        )?;

        for (resolution, decomposition) in component_data.decompositions.iter_mut().enumerate() {
            for sub_band in &mut decomposition.sub_bands {
                process_sub_band(
                    sub_band,
                    resolution as u16 + 1,
                    component_info,
                    &mut b_ctx,
                    &mut layer_buffer,
                )?;
            }
        }

        let idwt_output = idwt::apply(
            &component_data.first_ll_sub_band,
            &component_data.decompositions,
            component_info
                .coding_style_parameters
                .parameters
                .transformation,
        );

        // eprintln!("{:?}", component_samples.iter().map(|n| *n as i32).collect::<Vec<_>>());

        idw_output.push(idwt_output);
    }

    Ok(idw_output)
}

fn process_sub_band(
    sub_band: &mut SubBand,
    resolution: u16,
    component_info: &ComponentInfo,
    b_ctx: &mut BitplaneDecodeContext,
    layer_buffer: &mut Vec<u8>,
) -> Result<(), &'static str> {
    let dequantization_step =
        dequantization_factor(sub_band.sub_band_type, resolution, component_info);

    for precinct in &mut sub_band.precincts {
        for codeblock in &mut precinct.code_blocks {
            let num_bitplanes = {
                let (exponent, _) =
                    component_info.exponent_mantissa(sub_band.sub_band_type, resolution as u16);
                // Equation (E-2)
                component_info.quantization_info.guard_bits as u16 + exponent - 1
            };
            // eprintln!(
            //     "decoding block {}x{}",
            //     codeblock.area.width(),
            //     codeblock.area.height()
            // );
            bitplane::decode(
                codeblock,
                sub_band.sub_band_type,
                num_bitplanes,
                &component_info
                    .coding_style_parameters
                    .parameters
                    .code_block_style,
                b_ctx,
                layer_buffer,
            )?;

            // eprintln!("{:?}", codeblock.coefficients);

            // Copy the coefficients into the sub-band.

            let x_offset = codeblock.rect.x0 - sub_band.rect.x0;
            let y_offset = codeblock.rect.y0 - sub_band.rect.y0;

            for (y, in_row) in codeblock
                .coefficients
                .chunks_exact(codeblock.rect.width() as usize)
                .enumerate()
            {
                let out_row = &mut sub_band.coefficients[((y_offset + y as u32)
                    * sub_band.rect.width())
                    as usize
                    + x_offset as usize..];

                for (input, output) in in_row.iter().zip(out_row.iter_mut()) {
                    *output = *input as f32;

                    if let Some(q) = dequantization_step {
                        *output *= q;
                    }
                }
            }
        }
    }

    Ok(())
}

fn dequantization_factor(
    sub_band_type: SubBandType,
    resolution: u16,
    component_info: &ComponentInfo,
) -> Option<f32> {
    if component_info.quantization_info.quantization_style == QuantizationStyle::NoQuantization {
        return None;
    }

    let (exponent, mantissa) = component_info.exponent_mantissa(sub_band_type, resolution);

    let r_b = {
        let log_gain = match sub_band_type {
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

fn save_samples<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    channels: &mut [ChannelData],
    idwt_outputs: &mut [IDWTOutput],
) -> Option<()> {
    if header.global_coding_style.mct == MultipleComponentTransform::Used {
        if idwt_outputs.len() < 3 {
            return None;
        }

        let (s, _) = idwt_outputs.split_at_mut(3);
        let [s0, s1, s2] = s else { return None };
        let s0 = &mut s0.coefficients;
        let s1 = &mut s1.coefficients;
        let s2 = &mut s2.coefficients;

        let transform = header.component_infos[0].wavelet_transform();

        if transform != header.component_infos[1].wavelet_transform()
            || header.component_infos[1].wavelet_transform()
                != header.component_infos[2].wavelet_transform()
        {
            return None;
        }

        let len = s0.len();

        if len != s1.len() || s1.len() != s2.len() {
            return None;
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

    for ((idwt_output, component_info), channel_data) in idwt_outputs
        .iter_mut()
        .zip(header.component_infos.iter())
        .zip(channels.iter_mut())
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

    Some(())
}

fn parse_packet<'a>(
    tile: &TilePart<'a>,
    header: &Header,
    component_data: &mut [ComponentData<'a>],
    mut progression_iterator: impl Iterator<Item = ProgressionData>,
) -> Option<()> {
    let mut data = tile.data;

    while !data.is_empty() {
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

        let mut data_entries = vec![];
        let component_data = &mut component_data[progression_data.component as usize];

        // B.10.3 Zero length packet
        // "The first bit in the packet header denotes whether the packet has a length of zero
        // (empty packet). The value 0 indicates a zero length; no code-blocks are included in this
        // case. The value 1 indicates a non-zero length."
        if !zero_length {
            if resolution == 0 {
                parse_sub_band(
                    &mut component_data.first_ll_sub_band,
                    0,
                    &progression_data,
                    &mut reader,
                    &mut data_entries,
                )?;
            } else {
                let decomposition = &mut component_data.decompositions[resolution as usize - 1];

                for (sub_band_idx, sub_band) in decomposition.sub_bands.iter_mut().enumerate() {
                    parse_sub_band(
                        sub_band,
                        sub_band_idx,
                        &progression_data,
                        &mut reader,
                        &mut data_entries,
                    )?;
                }
            }
        }

        // TODO: What to do with the note below B.10.3?
        // TODO: Support multiple codeword segments (10.7.2)

        reader.read_stuff_bit_if_necessary()?;
        reader.align();
        let packet_data = reader.tail();

        let mut data_reader = Reader::new(packet_data);
        let mut total_length = 0;

        if header
            .global_coding_style
            .component_parameters
            .flags
            .uses_eph_marker()
            && data_reader.read_marker().ok()? != EPH
        {
            return None;
        }

        for (sub_band_idx, code_block_idx, length) in data_entries {
            let sub_band = if resolution == 0 {
                &mut component_data.first_ll_sub_band
            } else {
                &mut component_data.decompositions[resolution as usize - 1].sub_bands[sub_band_idx]
            };
            let precinct = &mut sub_band.precincts[progression_data.precinct as usize];
            let code_block = &mut precinct.code_blocks[code_block_idx];
            let layer = &mut code_block.layer_data[progression_data.layer_num as usize];

            *layer = data_reader.read_bytes(length as usize)?;
            total_length += length as usize;
        }

        data = data_reader.tail()?;
    }

    Some(())
}

fn parse_sub_band(
    sub_band: &mut SubBand,
    sub_band_idx: usize,
    progression_data: &ProgressionData,
    reader: &mut BitReader,
    data_entries: &mut Vec<(usize, usize, u32)>,
) -> Option<()> {
    let precinct = &mut sub_band.precincts[progression_data.precinct as usize];

    for (code_block_idx, code_block) in precinct.code_blocks.iter_mut().enumerate() {
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

        // eprintln!("code-block inclusion: {}", is_included);

        if !is_included {
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
            // eprintln!(
            //     "zero bit-plane information: {}",
            //     code_block.missing_bit_planes
            // );
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

        // eprintln!("number of coding passes: {}", added_coding_passes);

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
        data_entries.push((sub_band_idx, code_block_idx, length));

        // eprintln!("length(0) {}", length);
    }

    Some(())
}

fn build_component_data(
    tile: &Tile,
    header: &Header,
) -> Result<Vec<ComponentData<'static>>, &'static str> {
    let mut component_data = vec![];

    for (component_idx, component_info) in tile.component_info.iter().enumerate() {
        // TODO: IMprove this
        let mut ll_sub_band = None;
        let mut decompositions = vec![];

        for resolution in 0..component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels
        {
            let tile_instance = component_info.tile_instance(tile, resolution);

            if resolution == 0 {
                let decomposition_level = component_info
                    .coding_style_parameters
                    .parameters
                    .num_decomposition_levels;
                let rect = tile_instance.sub_band_rect(SubBandType::LowLow, decomposition_level);

                // eprintln!("making nLL for component {}", component_idx);
                // eprintln!(
                //     "Sub-band rect: [{},{} {}x{}], ll rect [{},{} {}x{}]",
                //     rect.x0,
                //     rect.y0,
                //     rect.width(),
                //     rect.height(),
                //     tile_instance.resolution_transformed_rect.x0,
                //     tile_instance.resolution_transformed_rect.y0,
                //     tile_instance.resolution_transformed_rect.width(),
                //     tile_instance.resolution_transformed_rect.height(),
                // );
                let precincts = build_precincts(&tile_instance, rect, header)?;

                ll_sub_band = Some(SubBand {
                    sub_band_type: SubBandType::LowLow,
                    rect,
                    precincts,
                    coefficients: vec![0.0; (rect.width() * rect.height()) as usize],
                })
            } else {
                let decomposition_level = component_info
                    .coding_style_parameters
                    .parameters
                    .num_decomposition_levels
                    - (resolution - 1);

                let build_sub_band = |sub_band_type: SubBandType| {
                    let rect = tile_instance.sub_band_rect(sub_band_type, decomposition_level);

                    let precincts = build_precincts(&tile_instance, rect, header)?;

                    Ok(SubBand {
                        sub_band_type,
                        rect,
                        precincts: precincts.clone(),
                        coefficients: vec![0.0; (rect.width() * rect.height()) as usize],
                    })
                };

                let decomposition = Decomposition {
                    sub_bands: [
                        build_sub_band(SubBandType::HighLow)?,
                        build_sub_band(SubBandType::LowHigh)?,
                        build_sub_band(SubBandType::HighHigh)?,
                    ],
                    rect: tile_instance.resolution_transformed_rect,
                };

                decompositions.push(decomposition);
            }
        }

        component_data.push(ComponentData {
            decompositions,
            first_ll_sub_band: ll_sub_band.unwrap(),
        })
    }

    Ok(component_data)
}

fn build_precincts(
    tile_instance: &TileInstance,
    sub_band_rect: IntRect,
    header: &Header,
) -> Result<Vec<Precinct<'static>>, &'static str> {
    let mut precincts = vec![];

    let num_precincts_y = tile_instance.num_precincts_y();
    let num_precincts_x = tile_instance.num_precincts_x();

    let mut ppx = tile_instance.ppx();
    let mut ppy = tile_instance.ppy();

    let mut y_start = (tile_instance.resolution_transformed_rect.y0 / (1 << ppy)) * (1 << ppy);
    let mut x_start = (tile_instance.resolution_transformed_rect.x0 / (1 << ppx)) * (1 << ppx);

    // TODO: I don't really understand where the specification mentions this is necessary. Just
    // copied this from Serenity.
    if tile_instance.resolution > 0 {
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

            let cb_width = tile_instance.code_block_width();
            let cb_height = tile_instance.code_block_height();

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

            // eprintln!(
            //     "Precinct rect: [{},{} {}x{}], num_code_blocks_wide: {}, num_code_blocks_high: {}",
            //     precinct_rect.x0,
            //     precinct_rect.y0,
            //     precinct_rect.width(),
            //     precinct_rect.height(),
            //     code_blocks_x,
            //     code_blocks_y
            // );

            let blocks = build_precinct_code_blocks(
                code_block_area,
                sub_band_rect,
                tile_instance,
                code_blocks_x,
                code_blocks_y,
                header.global_coding_style.num_layers,
            );

            let code_inclusion_tree = TagTree::new(code_blocks_x, code_blocks_y);
            let zero_bitplane_tree = TagTree::new(code_blocks_x, code_blocks_y);

            precincts.push(Precinct {
                area: precinct_rect,
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

fn build_precinct_code_blocks(
    code_block_area: IntRect,
    sub_band_rect: IntRect,
    tile_instance: &TileInstance,
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

        // eprintln!("num blocks: {:?}", code_blocks_y);
        // eprintln!("height: {:?}", code_block_height);

        for x_idx in 0..code_blocks_x {
            let area = IntRect::from_xywh(x, y, code_block_width, code_block_height)
                .intersect(sub_band_rect);

            // eprintln!(
            //     "Codeblock rect: [{},{} {}x{}]",
            //     area.x0,
            //     area.y0,
            //     area.width(),
            //     area.height(),
            // );

            blocks.push(CodeBlock {
                x_idx,
                y_idx,
                rect: area,
                layer_data: vec![&[]; num_layers as usize],
                has_been_included: false,
                missing_bit_planes: 0,
                l_block: 3,
                number_of_coding_passes: 0,
                coefficients: vec![],
            });

            x += code_block_width;
        }

        y += code_block_height;
    }

    blocks
}

pub(crate) trait BitReaderExt {
    fn read_packet_header_bits(&mut self, bit_size: u8) -> Option<u32>;
    fn read_stuff_bit_if_necessary(&mut self) -> Option<()>;
    fn peak_packet_header_bits(&mut self, bit_size: u8) -> Option<u32>;
}

impl BitReaderExt for BitReader<'_> {
    fn read_packet_header_bits(&mut self, bit_size: u8) -> Option<u32> {
        let mut bit = 0;

        for _ in 0..bit_size {
            self.read_stuff_bit_if_necessary()?;
            bit = (bit << 1) | self.read(1)?;
        }

        Some(bit)
    }

    fn read_stuff_bit_if_necessary(&mut self) -> Option<()> {
        // B.10.1: If the value of the byte is 0xFF, the next byte includes an extra zero bit
        // stuffed into the MSB.
        // Check if the next bit is at a new byte boundary.
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
