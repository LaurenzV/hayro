use crate::codestream::{Header, ProgressionOrder};
use crate::progression::{
    IteratorInput, ProgressionData, ProgressionIterator,
    ResolutionLevelLayerComponentPositionProgressionIterator,
};
use crate::tile::{IntRect, Tile, TileInstance, TilePart};
use hayro_common::bit::BitReader;

struct ComponentData<'a> {
    subbands: Vec<SubBands<'a>>,
}

enum SubBands<'a> {
    Lowest(SubBand<'a>),
    High(SubBand<'a>, SubBand<'a>, SubBand<'a>),
}

enum SubbandType {
    LowLow,
    LowHigh,
    HighLow,
    HighHigh,
}

struct SubBand<'a> {
    subband_type: SubbandType,
    precincts: Vec<Precinct<'a>>,
}

struct Precinct<'a> {
    area: IntRect,
    code_blocks: Vec<CodeBlock<'a>>,
}

struct CodeBlock<'a> {
    area: IntRect,
    layers: Vec<&'a [u8]>,
    coefficients: Vec<u8>,
}

pub(crate) fn process_tiles(tiles: &[Tile], header: &Header) -> Option<()> {
    for tile in tiles {
        let iter_input = IteratorInput::new(
            &tile,
            &header.component_infos,
            header.global_coding_style.num_layers,
        );

        match header.global_coding_style.progression_order {
            ProgressionOrder::ResolutionLayerComponentPosition => {
                let iter =
                    ResolutionLevelLayerComponentPositionProgressionIterator::new(iter_input);
                process_tile(&tile, header, iter)?;
            }
            _ => unimplemented!(),
        }
    }

    Some(())
}

fn process_tile<'a, T: ProgressionIterator<'a>>(
    tile: &Tile,
    header: &Header,
    mut iterator: T,
) -> Option<()> {
    let _ = build_component_data(tile, header);

    for tile_part in tile.tile_parts() {
        process_packet(&tile_part, header, &mut iterator)?;
    }

    Some(())
}

fn process_packet<'a, T: ProgressionIterator<'a>>(
    tile: &TilePart,
    header: &Header,
    mut iterator: &mut T,
) -> Option<()> {
    let mut reader = BitReader::new(&tile.data);

    // while let Some(ProgressionData {
    //     layer_num,
    //     resolution,
    //     component,
    //     precinct,
    // }) = iterator.next()
    // {}

    Some(())
}

fn build_component_data(tile: &Tile, header: &Header) -> Vec<ComponentData<'static>> {
    // let mut data = vec![];

    for component_info in &header.component_infos {
        let rect = component_info.scaled_rect(tile.rect);

        for resolution in 0..component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels
        {
            let tile_instance = component_info.tile_instance(&tile, resolution);

            build_precincts(&tile_instance);
        }
    }

    unimplemented!()
}

fn build_precincts(tile_instance: &TileInstance) -> Vec<Precinct<'static>> {
    let mut precincts = vec![];

    let precinct_width = tile_instance.precinct_width();
    let precinct_height = tile_instance.precinct_height();

    let mut y0 = tile_instance.dimensions.y0;

    let x1 = tile_instance.dimensions.x1;
    let y1 = tile_instance.dimensions.y1;

    for _ in 0..tile_instance.num_precincts_y() {
        let mut x0 = tile_instance.dimensions.x0;

        for _ in 0..tile_instance.num_precincts_x() {
            let width = u32::min(precinct_width, (x1 - x0));
            let height = u32::min(precinct_height, (y1 - y0));

            let precinct_rect = IntRect::from_xywh(x0, y0, width, height);

            let blocks = build_precinct_code_blocks(precinct_rect, &tile_instance);

            precincts.push(Precinct {
                area: precinct_rect,
                code_blocks: blocks,
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
) -> Vec<CodeBlock<'static>> {
    let mut blocks = vec![];

    let mut y = precinct_rect.y0;

    let code_block_width = tile_instance.code_block_width();
    let code_block_height = tile_instance.code_block_height();

    for _ in 0..tile_instance.code_blocks_y() {
        let mut x = precinct_rect.x0;

        for _ in 0..tile_instance.code_blocks_x() {
            let width = u32::min(code_block_width, precinct_rect.x1 - x);
            let height = u32::min(code_block_height, precinct_rect.y1 - y);

            let area = IntRect::from_xywh(x, y, width, height);

            blocks.push(CodeBlock {
                area,
                layers: vec![],
                coefficients: vec![],
            });

            x += code_block_width;
        }

        y += code_block_height;
    }

    blocks
}

trait BitReaderExt {
    fn read_packet_header_bit(&mut self, bit_size: u8) -> Option<u32>;
}

impl BitReaderExt for BitReader<'_> {
    fn read_packet_header_bit(&mut self, bit_size: u8) -> Option<u32> {
        let cur_byte_pos = self.byte_pos();
        let has_stuffing = self.cur_byte()? == 0xFF;

        let bit = self.read(bit_size)?;

        if self.byte_pos() != cur_byte_pos && has_stuffing {
            // B.10.1: If the value of the byte is 0xFF, the next byte includes an extra zero bit
            // stuffed into the MSB.
            let stuff_bit = self.read(1)?;
            assert_eq!(stuff_bit, 0);
        }

        Some(bit)
    }
}
