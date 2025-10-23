use crate::codestream::Header;
use crate::progression::{
    IteratorInput, ProgressionIterator, ResolutionLevelLayerComponentPositionProgressionIterator,
};
use crate::tile::{Tile, TilePart};
use hayro_common::bit::BitReader;

pub(crate) fn process_tiles(tiles: &[Tile], header: &Header) -> Option<()> {
    for tile in tiles {
        for tile_part in tile.tile_parts() {
            process_packet(&tile_part, header)?;
        }
    }

    Some(())
}

fn process_packet(tile: &TilePart, header: &Header) -> Option<()> {
    let input = IteratorInput::new(
        tile,
        &header.component_infos,
        header.global_coding_style.num_layers,
    );
    let iter = ResolutionLevelLayerComponentPositionProgressionIterator::new(input);

    let mut reader = BitReader::new(&tile.data);
    let zero_length = reader.read(1)?;

    Some(())
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
