use hayro_common::byte::Reader;
use crate::codestream::{markers, CodingStyleInfo, Header, QuantizationInfo, ReaderExt};

#[derive(Clone, Debug)]
pub(crate) struct Tile<'a> {
    pub(crate) parts: Vec<TilePart<'a>>
}

impl Tile<'_> {
    fn new() -> Tile<'static> {
        Tile {
            parts: vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TilePart<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) cod_components: Vec<CodingStyleInfo>,
    pub(crate) qcd_components: Vec<QuantizationInfo>,
}

pub(crate) fn read_tiles<'a>(reader: &mut Reader<'a>, main_header: &Header) -> Result<Vec<Tile<'a>>, &'static str> {
    let mut parsed_tile_parts = {
        let mut buf = vec![];
        buf.push(read_tile_part(reader, &main_header).ok_or("failed to read first tile part")?);

        while reader.peek_marker() == Some(markers::SOT) {
            buf.push(read_tile_part(reader, &main_header).ok_or("failed to read a tile part")?);
        }

        if reader.read_marker()? != markers::EOC {
            return Err("invalid marker: expected EOC marker");
        }

        buf.sort_by(|t1, t2| (t1.tile_index, t1.tile_part_index).cmp(&(t2.tile_index, t2.tile_part_index)));
        
        buf
    };
    
    let mut tiles = vec![Tile::new(); main_header.size_data.num_tiles() as usize];
    
    for tile_part in parsed_tile_parts {
        let cur_tile = tiles.get_mut(tile_part.tile_index as usize).ok_or("tile part had invalid tile index")?;
        
        cur_tile.parts.push(TilePart {
            data: tile_part.data,
            cod_components: tile_part.cod_components.clone(),
            qcd_components: tile_part.qcd_components.clone(),
        });
    }
    
    Ok(tiles)
}

struct ParsedTilePart<'a> {
    tile_index: u16,
    tile_part_index: u8,
    cod_components: Vec<CodingStyleInfo>,
    qcd_components: Vec<QuantizationInfo>,
    data: &'a [u8],
}

fn read_tile_part<'a>(reader: &mut Reader<'a>, main_header: &Header) -> Option<ParsedTilePart<'a>> {
    if reader.read_marker().ok()? != markers::SOT {
        return None;
    }

    let (mut tile_part_reader, header) = {
        let sot_marker = sot_marker(reader)?;
        let data = if sot_marker.tile_part_length == 0 {
            // Data goes until EOC.
            let data = reader.tail()?;
            reader.jump_to_end();

            data
        } else {
            // Subtract 12 to account for the marker length.
            let length = (sot_marker.tile_part_length as usize).checked_sub(12)?;

            let data = reader.tail()?.get(..length)?;
            // Skip to the very end in the original reader.
            reader.skip_bytes(length)?;

            data
        };

        (Reader::new(data), sot_marker)
    };

    loop {
        match tile_part_reader.peek_marker()? {
            markers::SOD => {
                tile_part_reader.read_marker().ok()?;
                break;
            }
            markers::EOC => break,
            m => {
                panic!("marker: {}", markers::to_string(m));
            }
        }
    }

    Some(ParsedTilePart {
        data: tile_part_reader.tail()?,
        tile_index: header.tile_index,
        tile_part_index: header.tile_part_index,
        cod_components: main_header.cod_components.clone(),
        qcd_components: main_header.qcd_components.clone(),
    })
}

struct TilePartHeader {
    tile_index: u16,
    tile_part_length: u32,
    tile_part_index: u8,
    num_tile_parts: u8,
}

/// SOT marker (A.4.2).
pub(crate) fn sot_marker(reader: &mut Reader) -> Option<TilePartHeader> {
    // Length.
    let _ = reader.read_u16()?;

    let tile_index = reader.read_u16()?;
    let tile_part_length = reader.read_u32()?;
    let tile_part_index = reader.read_byte()?;
    let num_tile_parts = reader.read_byte()?;

    Some(TilePartHeader {
        tile_index,
        tile_part_length,
        tile_part_index,
        num_tile_parts,
    })
}