use crate::codestream::{Header, ProgressionOrder};
use crate::progression::{
    IteratorInput, ProgressionData, ProgressionIterator,
    ResolutionLevelLayerComponentPositionProgressionIterator,
};
use crate::tile::{IntRect, Tile, TilePart};
use hayro_common::bit::BitReader;

struct ComponentData<'a> {
    subbands: Vec<SubBands<'a>>
}

enum SubBands<'a> {
    Lowest(SubBand<'a>),
    High(SubBand<'a>, SubBand<'a>, SubBand<'a>)
}

enum SubbandType {
    LowLow,
    LowHigh,
    HighLow,
    HighHigh
}

struct SubBand<'a> {
    subband_type: SubbandType,
    precincts: Vec<Precinct<'a>>
}

struct Precinct<'a> {
    area: IntRect,
    code_blocks: Vec<CodeBlock<'a>>
}

struct CodeBlock<'a> {
    layers: Vec<&'a [u8]>,
    coefficients: Vec<u8>
}

pub(crate) fn process_tiles(tiles: &[Tile], header: &Header) -> Option<()> {
    for tile in tiles {
        for tile_part in tile.tile_parts() {
            let iter_input = IteratorInput::new(
                &tile_part,
                &header.component_infos,
                header.global_coding_style.num_layers,
            );

            match header.global_coding_style.progression_order {
                ProgressionOrder::ResolutionLayerComponentPosition => {
                    let iter =
                        ResolutionLevelLayerComponentPositionProgressionIterator::new(iter_input);
                    process_packet(&tile_part, header, iter)?;
                }
                _ => unimplemented!(),
            }
        }
    }

    Some(())
}

fn process_packet<'a, T: ProgressionIterator<'a>>(
    tile: &TilePart,
    header: &Header,
    mut iterator: T,
) -> Option<()> {
    let mut reader = BitReader::new(&tile.data);

    while let Some(ProgressionData {
        layer_num,
        resolution,
        component,
        precinct,
    }) = iterator.next()
    {}

    Some(())
}

fn build_component_data(tile: &Tile, header: &Header) -> Vec<ComponentData<'static>> {
    unimplemented!();
    // for component_info in &header.component_infos {
    //     let rect = component_info.scaled_rect(tile.rect);
    //     
    //     for resolution in 0..component_info.coding_style_parameters.parameters.num_resolution_levels
    // }
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
