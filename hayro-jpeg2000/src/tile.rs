use crate::codestream::{
    CodingStyleInfo, ComponentInfo, Header, QuantizationInfo, ReaderExt, SizeData, markers,
};
use hayro_common::byte::Reader;

#[derive(Clone, Copy, Debug)]
pub(crate) struct IntRect {
    pub(crate) x0: u32,
    pub(crate) y0: u32,
    pub(crate) x1: u32,
    pub(crate) y1: u32,
}

impl IntRect {
    pub(crate) fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub(crate) fn width(&self) -> u32 {
        // See B-11.
        self.x1 - self.x0
    }

    pub(crate) fn height(&self) -> u32 {
        // See B-11.
        self.y1 - self.y0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Tile<'a> {
    size_data: &'a SizeData,
    tile_part_infos: Vec<TilePartInfo<'a>>,
    raw_coords: IntRect,
}

impl<'a> Tile<'a> {
    fn new(idx: u32, size_data: &'a SizeData) -> Tile<'a> {
        // See B-6.
        let p = idx % size_data.num_x_tiles();
        // I believe the `ceil` in the spec should be a `floor` instead.
        let q = (idx as f64 / size_data.num_x_tiles() as f64).floor() as u32;

        // See B-7, B-8, B-9 and B-10.
        let x0 = u32::max(
            size_data.tile_x_offset + p * size_data.tile_width,
            size_data.image_area_x_offset,
        );
        let y0 = u32::max(
            size_data.tile_y_offset + q * size_data.tile_height,
            size_data.image_area_y_offset,
        );
        let x1 = u32::min(
            size_data.tile_x_offset + (p + 1) * size_data.tile_width,
            size_data.reference_grid_width,
        );
        let y1 = u32::min(
            size_data.tile_y_offset + (q + 1) * size_data.tile_height,
            size_data.reference_grid_height,
        );

        let coordinates = IntRect::new(x0, y0, x1, y1);

        Tile {
            size_data,
            tile_part_infos: vec![],
            raw_coords: coordinates,
        }
    }
    
    pub(crate) fn tile_parts(&self) -> impl Iterator<Item = TilePart<'_, '_>> {
        self.tile_part_infos.iter().map(|t| TilePart {
            data: t.data,
            tile: self,
        })
    }

    fn raw_tile_coords(&self) -> IntRect {
        self.raw_coords
    }
    
    /// Compute the coordinates of the tiles for the resolution of the given component.
    fn tile_coords(&self, info: &ComponentInfo) -> IntRect {
        if info.horizontal_resolution == 1 && info.vertical_resolution == 1 {
            self.raw_coords
        } else {
            // As described in B-12.
            let x0 = self
                .raw_coords
                .x0
                .div_ceil(info.horizontal_resolution as u32);
            let y0 = self.raw_coords.y0.div_ceil(info.vertical_resolution as u32);
            let x1 = self
                .raw_coords
                .x1
                .div_ceil(info.horizontal_resolution as u32);
            let y1 = self.raw_coords.y1.div_ceil(info.vertical_resolution as u32);

            IntRect::new(x0, y0, x1, y1)
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TilePartInfo<'a> {
    pub(crate) data: &'a [u8],
}

#[derive(Clone, Debug)]
pub(crate) struct TilePart<'a, 'b> {
    pub(crate) data: &'a [u8],
    pub(crate) tile: &'b Tile<'b>
}

pub(crate) fn read_tiles<'a>(
    reader: &mut Reader<'a>,
    main_header: &'a Header,
) -> Result<Vec<Tile<'a>>, &'static str> {
    let mut parsed_tile_parts = {
        let mut buf = vec![];
        buf.push(read_tile_part(reader, &main_header).ok_or("failed to read first tile part")?);

        while reader.peek_marker() == Some(markers::SOT) {
            buf.push(read_tile_part(reader, &main_header).ok_or("failed to read a tile part")?);
        }

        if reader.read_marker()? != markers::EOC {
            return Err("invalid marker: expected EOC marker");
        }

        buf.sort_by(|t1, t2| {
            (t1.tile_index, t1.tile_part_index).cmp(&(t2.tile_index, t2.tile_part_index))
        });

        buf
    };

    let mut tiles = (0..main_header.size_data.num_tiles() as usize)
        .into_iter()
        .map(|idx| Tile::new(idx as u32, &main_header.size_data))
        .collect::<Vec<_>>();

    for tile_part in parsed_tile_parts {
        let cur_tile = tiles
            .get_mut(tile_part.tile_index as usize)
            .ok_or("tile part had invalid tile index")?;

        cur_tile.tile_part_infos.push(TilePartInfo {
            data: tile_part.data,
        });
    }

    Ok(tiles)
}

struct ParsedTilePart<'a> {
    tile_index: u16,
    tile_part_index: u8,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test case for the example in B.4.
    #[test]
    fn test_jpeg2000_standard_example_b4() {
        let size_data = SizeData {
            reference_grid_width: 1432,
            reference_grid_height: 954,
            image_area_x_offset: 152,
            image_area_y_offset: 234,
            tile_width: 396,
            tile_height: 297,
            tile_x_offset: 0,
            tile_y_offset: 0,
            components: vec![
                ComponentInfo {
                    precision: 8,
                    is_signed: false,
                    horizontal_resolution: 1,
                    vertical_resolution: 1,
                },
                ComponentInfo {
                    precision: 8,
                    is_signed: false,
                    horizontal_resolution: 2,
                    vertical_resolution: 2,
                },
            ],
        };

        assert_eq!(size_data.image_width(), 1280);
        assert_eq!(size_data.image_height(), 720);

        assert_eq!(size_data.num_x_tiles(), 4);
        assert_eq!(size_data.num_y_tiles(), 4);
        assert_eq!(size_data.num_tiles(), 16);

        let component_0 = &size_data.components[0];
        let component_1 = &size_data.components[1];

        let tile_0_0 = Tile::new(0, &size_data);
        let coords_0_0 = tile_0_0.tile_coords(component_0);
        assert_eq!(coords_0_0.x0, 152);
        assert_eq!(coords_0_0.y0, 234);
        assert_eq!(coords_0_0.x1, 396);
        assert_eq!(coords_0_0.y1, 297);
        assert_eq!(coords_0_0.width(), 244);
        assert_eq!(coords_0_0.height(), 63);

        let tile_1_0 = Tile::new(1, &size_data);
        let coords_1_0 = tile_1_0.tile_coords(component_0);
        assert_eq!(coords_1_0.x0, 396);
        assert_eq!(coords_1_0.y0, 234);
        assert_eq!(coords_1_0.x1, 792);
        assert_eq!(coords_1_0.y1, 297);
        assert_eq!(coords_1_0.width(), 396);
        assert_eq!(coords_1_0.height(), 63);

        let tile_0_1 = Tile::new(4, &size_data);
        let coords_0_1 = tile_0_1.tile_coords(component_0);
        assert_eq!(coords_0_1.x0, 152);
        assert_eq!(coords_0_1.y0, 297);
        assert_eq!(coords_0_1.x1, 396);
        assert_eq!(coords_0_1.y1, 594);
        assert_eq!(coords_0_1.width(), 244);
        assert_eq!(coords_0_1.height(), 297);

        let tile_1_1 = Tile::new(5, &size_data);
        let coords_1_1 = tile_1_1.tile_coords(component_0);
        assert_eq!(coords_1_1.x0, 396);
        assert_eq!(coords_1_1.y0, 297);
        assert_eq!(coords_1_1.x1, 792);
        assert_eq!(coords_1_1.y1, 594);
        assert_eq!(coords_1_1.width(), 396);
        assert_eq!(coords_1_1.height(), 297);

        let tile_3_3 = Tile::new(15, &size_data);
        let coords_3_3 = tile_3_3.tile_coords(component_0);
        assert_eq!(coords_3_3.x0, 1188);
        assert_eq!(coords_3_3.y0, 891);
        assert_eq!(coords_3_3.x1, 1432);
        assert_eq!(coords_3_3.y1, 954);
        assert_eq!(coords_3_3.width(), 244);
        assert_eq!(coords_3_3.height(), 63);

        let tile_0_0_comp1 = tile_0_0.tile_coords(component_1);
        assert_eq!(tile_0_0_comp1.x0, 76);
        assert_eq!(tile_0_0_comp1.y0, 117);
        assert_eq!(tile_0_0_comp1.x1, 198);
        assert_eq!(tile_0_0_comp1.y1, 149);
        assert_eq!(tile_0_0_comp1.width(), 122);
        assert_eq!(tile_0_0_comp1.height(), 32);

        let tile_1_0_comp1 = tile_1_0.tile_coords(component_1);
        assert_eq!(tile_1_0_comp1.x0, 198);
        assert_eq!(tile_1_0_comp1.y0, 117);
        assert_eq!(tile_1_0_comp1.x1, 396);
        assert_eq!(tile_1_0_comp1.y1, 149);
        assert_eq!(tile_1_0_comp1.width(), 198);
        assert_eq!(tile_1_0_comp1.height(), 32);

        let tile_0_1_comp1 = tile_0_1.tile_coords(component_1);
        assert_eq!(tile_0_1_comp1.x0, 76);
        assert_eq!(tile_0_1_comp1.y0, 149);
        assert_eq!(tile_0_1_comp1.x1, 198);
        assert_eq!(tile_0_1_comp1.y1, 297);
        assert_eq!(tile_0_1_comp1.width(), 122);
        assert_eq!(tile_0_1_comp1.height(), 148);

        let tile_1_1_comp1 = tile_1_1.tile_coords(component_1);
        assert_eq!(tile_1_1_comp1.x0, 198);
        assert_eq!(tile_1_1_comp1.y0, 149);
        assert_eq!(tile_1_1_comp1.x1, 396);
        assert_eq!(tile_1_1_comp1.y1, 297);
        assert_eq!(tile_1_1_comp1.width(), 198);
        assert_eq!(tile_1_1_comp1.height(), 148);

        let tile_2_1 = Tile::new(6, &size_data);
        let tile_2_1_comp1 = tile_2_1.tile_coords(component_1);
        assert_eq!(tile_2_1_comp1.x0, 396);
        assert_eq!(tile_2_1_comp1.y0, 149);
        assert_eq!(tile_2_1_comp1.x1, 594);
        assert_eq!(tile_2_1_comp1.y1, 297);
        assert_eq!(tile_2_1_comp1.width(), 198);
        assert_eq!(tile_2_1_comp1.height(), 148);

        assert_eq!(tile_1_1_comp1.width(), tile_2_1_comp1.width());
        assert_eq!(tile_1_1_comp1.height(), tile_2_1_comp1.height());
    }
}
