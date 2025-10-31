use crate::bitmap::{Bitmap, ChannelContainer, ChannelData};
use crate::codestream::{
    Header, MultipleComponentTransform, ProgressionOrder, QuantizationStyle, WaveletTransform,
};
use crate::progression::{IteratorInput, LayerResolutionLevelComponentPositionProgressionIterator, ProgressionIterator, ResolutionLevelLayerComponentPositionProgressionIterator};
use crate::tag_tree::TagTree;
use crate::tile::{IntRect, Tile, TileInstance, TilePart};
use crate::{ChannelType, bitplane, idwt};
use hayro_common::bit::BitReader;
use hayro_common::byte::Reader;

struct ComponentData<'a> {
    subbands: Vec<Vec<SubBand<'a>>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SubbandType {
    LowLow,
    LowHigh,
    HighLow,
    HighHigh,
}

#[derive(Clone)]
pub(crate) struct SubBand<'a> {
    pub(crate) subband_type: SubbandType,
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
    pub(crate) area: IntRect,
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

pub(crate) fn process_tiles(tiles: &[Tile], header: &Header) -> Option<Vec<ChannelData>> {
    let mut channels = vec![];

    for (idx, info) in header.component_infos.iter().enumerate() {
        channels.push(ChannelData {
            container: ChannelContainer::U8(vec![
                0;
                (header.size_data.reference_grid_width * header.size_data.reference_grid_height)
                    as usize
            ]),
            is_alpha: header
                .metadata
                .channel_definitions
                .get(idx)
                .map(|c| c.channel_type == ChannelType::Opacity)
                .unwrap_or(false),
            bit_depth: info.size_info.precision,
        })
    }

    for tile in tiles {
        let iter_input = IteratorInput::new(
            tile,
            &header.component_infos,
            header.global_coding_style.num_layers,
        );

        let mut samples = match header.global_coding_style.progression_order {
            ProgressionOrder::ResolutionLayerComponentPosition => {
                let iter =
                    ResolutionLevelLayerComponentPositionProgressionIterator::new(iter_input);
                process_tile(tile, header, iter)?
            }
            _ => unimplemented!(),
        };

        save_samples(tile, header, &mut channels, &mut samples)?;
    }

    Some(channels)
}

fn process_tile<'a, T: ProgressionIterator<'a>>(
    tile: &'a Tile<'a>,
    header: &Header,
    mut iterator: T,
) -> Option<Vec<Vec<f32>>> {
    let mut component_data = build_component_data(tile, header);

    for tile_part in tile.tile_parts() {
        parse_packet(&tile_part, header, &mut component_data, &mut iterator)?;
    }

    let mut samples = vec![];

    for (component_data, component_info) in
        component_data.iter_mut().zip(header.component_infos.iter())
    {
        for resolution_level in &mut component_data.subbands {
            for subband in resolution_level {
                for precinct in &mut subband.precincts {
                    for codeblock in &mut precinct.code_blocks {
                        eprintln!(
                            "decoding block {}x{}",
                            codeblock.area.width(),
                            codeblock.area.height()
                        );
                        bitplane::decode(
                            codeblock,
                            subband.subband_type,
                            &component_info
                                .coding_style_parameters
                                .parameters
                                .code_block_style,
                        )?;

                        if component_info.quantization_info.quantization_style
                            != QuantizationStyle::NoQuantization
                        {
                            panic!("quantization not implemented yet.");
                        }

                        // Copy the coefficients into the subband.

                        let x_offset = codeblock.area.x0 - subband.rect.x0;
                        let y_offset = codeblock.area.y0 - subband.rect.y0;

                        for (y, in_row) in codeblock
                            .coefficients
                            .chunks_exact(codeblock.area.width() as usize)
                            .enumerate()
                        {
                            let out_row = &mut subband.coefficients[((y_offset + y as u32)
                                * subband.rect.width())
                                as usize
                                + x_offset as usize..];

                            for (input, output) in in_row.iter().zip(out_row.iter_mut()) {
                                *output = *input as f32;
                            }
                        }
                    }
                }
            }
        }

        samples.push(idwt::apply(
            &component_data.subbands,
            component_info
                .coding_style_parameters
                .parameters
                .transformation,
        ));
    }

    Some(samples)
}

fn save_samples<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
    channels: &mut [ChannelData],
    samples: &mut [Vec<f32>],
) -> Option<()> {
    if header.global_coding_style.mct == MultipleComponentTransform::Used {
        let [s0, s1, s2] = samples else { return None };

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
                unimplemented!()
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

    for ((samples, component_info), channel_data) in samples
        .iter_mut()
        .zip(header.component_infos.iter())
        .zip(channels.iter_mut())
    {
        for sample in samples.iter_mut() {
            *sample += (1 << component_info.size_info.precision - 1) as f32;
        }

        let tile_x_offset = tile.rect.x0;
        let tile_y_offset = tile.rect.y0;

        match &mut channel_data.container {
            ChannelContainer::U8(c) => {
                for y in tile_y_offset..(tile_y_offset + tile.rect.height()) {
                    let output = &mut c
                        [(y * header.size_data.reference_grid_width + tile_x_offset) as usize..]
                        [..tile.rect.width() as usize];
                    let input =
                        &samples[(y * tile.rect.width()) as usize..][..tile.rect.width() as usize];

                    for (i, o) in input.iter().zip(output.iter_mut()) {
                        *o = *i as u8;
                    }
                }
            }
            _ => unimplemented!(),
        }
    }

    Some(())
}

fn parse_packet<'a, T: ProgressionIterator<'a>>(
    tile: &TilePart<'a>,
    header: &Header,
    component_data: &mut [ComponentData<'a>],
    progression_iterator: &mut T,
) -> Option<()> {
    let mut data = tile.data;

    while !data.is_empty() {
        let mut reader = BitReader::new(data);

        let progression_data = progression_iterator.next()?;
        let zero_length = reader.read_packet_header_bits(1)?;

        // B.10.3 Zero length packet
        // "The first bit in the packet header denotes whether the packet has a length of zero
        // (empty packet). The value 0 indicates a zero length; no code-blocks are included in this
        // case. The value 1 indicates a non-zero length.
        if zero_length == 0 {
            continue;
        }

        let mut data_entries = vec![];

        // TODO: What to do with the note below B.10.3?

        let component = &mut component_data[progression_data.component as usize];
        let sub_bands = &mut component.subbands[progression_data.resolution as usize];

        for (sub_band_idx, sub_band) in sub_bands.iter_mut().enumerate() {
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
                        &mut reader,
                        progression_data.layer_num as u32 + 1,
                    )? <= progression_data.layer_num as u32
                };

                eprintln!("code-block inclusion: {}", is_included);

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
                        &mut reader,
                        u32::MAX,
                    )? as u8;
                    eprintln!(
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

                eprintln!("number of coding passes: {}", added_coding_passes);

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

                eprintln!("length(0) {}", length);
            }
        }

        // TODO: Support multiple codeword segments (10.7.2)

        reader.align();
        let packet_data = reader.tail();

        let mut data_reader = Reader::new(packet_data);
        let mut total_length = 0;

        for (sub_band_idx, code_block_idx, length) in data_entries {
            let sub_band = &mut sub_bands[sub_band_idx];
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

fn build_component_data(tile: &Tile, header: &Header) -> Vec<ComponentData<'static>> {
    let mut component_data = vec![];

    for component_info in &header.component_infos {
        let mut bands = vec![];

        for resolution in 0..component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels
        {
            let tile_instance = component_info.tile_instance(tile, resolution);

            eprintln!("resolution: {}", resolution);

            if resolution == 0 {
                let decomposition_level = component_info
                    .coding_style_parameters
                    .parameters
                    .num_decomposition_levels;
                let rect = tile_instance.sub_band_rect(SubbandType::LowLow, decomposition_level);

                eprintln!(
                    "Sub-band rect: {:?}, ll rect: {:?}",
                    rect, tile_instance.resolution_transformed_rect
                );
                let precincts = build_precincts(&tile_instance, rect, header);

                bands.push(vec![SubBand {
                    subband_type: SubbandType::LowLow,
                    rect,
                    precincts,
                    coefficients: vec![0.0; (rect.width() * rect.height()) as usize],
                }]);
            } else {
                let decomposition_level = component_info
                    .coding_style_parameters
                    .parameters
                    .num_decomposition_levels
                    - (resolution - 1);

                let mut sub_bands = vec![];

                for sb_type in [
                    SubbandType::HighLow,
                    SubbandType::LowHigh,
                    SubbandType::HighHigh,
                ] {
                    let rect = tile_instance.sub_band_rect(sb_type, decomposition_level);
                    let precincts = build_precincts(&tile_instance, rect, header);
                    eprintln!(
                        "Sub-band rect: {:?}, ll rect: {:?}",
                        rect, tile_instance.resolution_transformed_rect
                    );
                    sub_bands.push(SubBand {
                        subband_type: sb_type,
                        rect,
                        precincts: precincts.clone(),
                        coefficients: vec![0.0; (rect.width() * rect.height()) as usize],
                    })
                }

                bands.push(sub_bands);
            }
        }

        component_data.push(ComponentData { subbands: bands })
    }

    component_data
}

fn build_precincts(
    tile_instance: &TileInstance,
    sub_band_rect: IntRect,
    header: &Header,
) -> Vec<Precinct<'static>> {
    let mut precincts = vec![];

    let precinct_width = tile_instance.precinct_width();
    let precinct_height = tile_instance.precinct_height();

    let mut y0 = tile_instance.resolution_transformed_rect.y0;

    for _ in 0..tile_instance.num_precincts_y() {
        let mut x0 = tile_instance.resolution_transformed_rect.x0;

        for _ in 0..tile_instance.num_precincts_x() {
            let precinct_rect = IntRect::from_xywh(x0, y0, precinct_width, precinct_height);

            let code_blocks_x = sub_band_rect
                .width()
                .div_ceil(tile_instance.code_block_width());
            let code_blocks_y = sub_band_rect
                .height()
                .div_ceil(tile_instance.code_block_height());

            let blocks = build_precinct_code_blocks(
                precinct_rect,
                tile_instance,
                sub_band_rect,
                code_blocks_y,
                code_blocks_x,
                header.global_coding_style.num_layers,
            );

            eprintln!(
                "Precinct rect: {:?}, code blocks width: {code_blocks_x}, code blocks height: {code_blocks_y}",
                precinct_rect
            );

            precincts.push(Precinct {
                area: precinct_rect,
                code_blocks: blocks,
                code_inclusion_tree: TagTree::new(code_blocks_x, code_blocks_y),
                zero_bitplane_tree: TagTree::new(code_blocks_x, code_blocks_y),
            });

            x0 += precinct_width;
        }

        y0 += precinct_height;
    }

    precincts
}

fn build_precinct_code_blocks(
    precinct_rect: IntRect,
    tile_instance: &TileInstance,
    sub_band_rect: IntRect,
    code_blocks_x: u32,
    code_blocks_y: u32,
    num_layers: u16,
) -> Vec<CodeBlock<'static>> {
    let mut blocks = vec![];

    let mut y = precinct_rect.y0;

    let code_block_width = tile_instance.code_block_width();
    let code_block_height = tile_instance.code_block_height();

    for y_idx in 0..code_blocks_y {
        let mut x = precinct_rect.x0;

        // eprintln!("num blocks: {:?}", code_blocks_y);
        // eprintln!("height: {:?}", code_block_height);

        for x_idx in 0..code_blocks_x {
            // eprintln!("{} {} {}", precinct_rect.y0, y, precinct_rect.y1);
            let width = u32::min(code_block_width, precinct_rect.x1 - x);
            let height = u32::min(code_block_height, precinct_rect.y1 - y);

            let area = IntRect::from_xywh(x, y, width, height).intersect(sub_band_rect);

            blocks.push(CodeBlock {
                x_idx,
                y_idx,
                area,
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
    fn peak_packet_header_bits(&mut self, bit_size: u8) -> Option<u32>;
}

impl BitReaderExt for BitReader<'_> {
    fn read_packet_header_bits(&mut self, bit_size: u8) -> Option<u32> {
        let mut bit = 0;

        for _ in 0..bit_size {
            // B.10.1: If the value of the byte is 0xFF, the next byte includes an extra zero bit
            // stuffed into the MSB.
            // Check if the next bit is at a new byte boundary.
            if self.bit_pos() == 0 && self.byte_pos() > 0 {
                let last_byte = self.data[self.byte_pos() - 1];

                if last_byte == 0xff {
                    let stuff_bit = self.read(1)?;

                    assert_eq!(stuff_bit, 0, "invalid stuffing bit");
                }
            }

            bit = (bit << 1) | self.read(1)?;
        }

        Some(bit)
    }

    fn peak_packet_header_bits(&mut self, bit_size: u8) -> Option<u32> {
        self.clone().read_packet_header_bits(bit_size)
    }
}
