use crate::codestream::{ComponentInfo, Header, ReaderExt, SizeData, markers, skip_marker_segment};
use crate::packet::SubBandType;
use crate::rect::IntRect;
use hayro_common::byte::Reader;

#[derive(Clone, Debug)]
pub(crate) struct Tile<'a> {
    pub(crate) tile_parts: Vec<TilePart<'a>>,
    pub(crate) component_info: Vec<ComponentInfo>,
    pub(crate) rect: IntRect,
}

impl<'a> Tile<'a> {
    fn new(idx: u32, size_data: &SizeData) -> Tile<'a> {
        let raw_coords = size_data.tile_coords(idx);

        Tile {
            tile_parts: vec![],
            component_info: vec![],
            rect: raw_coords,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TilePart<'a> {
    pub(crate) data: &'a [u8],
}

pub(crate) fn read_tiles<'a>(
    reader: &mut Reader<'a>,
    main_header: &'a Header,
) -> Result<Vec<Tile<'a>>, &'static str> {
    let parsed_tile_parts = {
        let mut buf = vec![];
        read_tile_part(reader, main_header, &mut buf)?;

        while reader.peek_marker() == Some(markers::SOT) {
            read_tile_part(reader, main_header, &mut buf)?;
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
        .map(|idx| Tile::new(idx as u32, &main_header.size_data))
        .collect::<Vec<_>>();

    for tile_part in parsed_tile_parts {
        let cur_tile = tiles
            .get_mut(tile_part.tile_index as usize)
            .ok_or("tile part had invalid tile index")?;

        if tile_part.tile_part_index == 0 {
            cur_tile.component_info = tile_part.component_infos.clone();
        }

        cur_tile.tile_parts.push(TilePart {
            data: tile_part.data,
        });
    }

    Ok(tiles)
}

pub(crate) struct TileInstance<'a> {
    pub(crate) resolution: u16,
    pub(crate) component_info: &'a ComponentInfo,
    pub(crate) tile_component_rect: IntRect,
    pub(crate) resolution_transformed_rect: IntRect,
}

impl<'a> TileInstance<'a> {
    pub(crate) fn ppx(&self) -> u8 {
        self.component_info
            .coding_style_parameters
            .parameters
            .precinct_exponents[self.resolution as usize]
            .0
    }

    pub(crate) fn ppy(&self) -> u8 {
        self.component_info
            .coding_style_parameters
            .parameters
            .precinct_exponents[self.resolution as usize]
            .1
    }

    pub(crate) fn sub_band_rect(
        &self,
        sub_band_type: SubBandType,
        decomposition_level: u16,
    ) -> IntRect {
        // Formula B-15.

        let xo_b = if matches!(sub_band_type, SubBandType::HighLow | SubBandType::HighHigh) {
            1
        } else {
            0
        };
        let yo_b = if matches!(sub_band_type, SubBandType::LowHigh | SubBandType::HighHigh) {
            1
        } else {
            0
        };

        let numerator_x = 2u32.pow(decomposition_level as u32 - 1) * xo_b;
        let numerator_y = 2u32.pow(decomposition_level as u32 - 1) * yo_b;
        let denominator = 2u32.pow(decomposition_level as u32);

        let tbx_0 = self
            .tile_component_rect
            .x0
            .saturating_sub(numerator_x)
            .div_ceil(denominator);
        let tbx_1 = self
            .tile_component_rect
            .x1
            .saturating_sub(numerator_x)
            .div_ceil(denominator);

        let tby_0 = self
            .tile_component_rect
            .y0
            .saturating_sub(numerator_y)
            .div_ceil(denominator);
        let tby_1 = self
            .tile_component_rect
            .y1
            .saturating_sub(numerator_y)
            .div_ceil(denominator);

        IntRect::from_ltrb(tbx_0, tby_0, tbx_1, tby_1)
    }

    pub(crate) fn num_precincts_x(&self) -> u32 {
        // See B-16.
        let IntRect { x0, x1, .. } = self.resolution_transformed_rect;

        if x0 == x1 {
            0
        } else {
            x1.div_ceil(2u32.pow(self.ppx() as u32)) - x0 / 2u32.pow(self.ppx() as u32)
        }
    }

    pub(crate) fn num_precincts_y(&self) -> u32 {
        // See B-16.
        let IntRect { y0, y1, .. } = self.resolution_transformed_rect;

        if y0 == y1 {
            0
        } else {
            y1.div_ceil(2u32.pow(self.ppy() as u32)) - y0 / 2u32.pow(self.ppy() as u32)
        }
    }

    pub(crate) fn num_precincts(&self) -> u32 {
        self.num_precincts_x() * self.num_precincts_y()
    }

    pub(crate) fn code_block_width(&self) -> u32 {
        // See B-17.
        let xcb = self
            .component_info
            .coding_style_parameters
            .parameters
            .code_block_width;

        let xcb = if self.resolution > 0 {
            u8::min(xcb, self.ppx() - 1)
        } else {
            u8::min(xcb, self.ppx())
        };

        2u32.pow(xcb as u32)
    }

    pub(crate) fn code_block_height(&self) -> u32 {
        // See B-18.
        let ycb = self
            .component_info
            .coding_style_parameters
            .parameters
            .code_block_height;

        let ycb = if self.resolution > 0 {
            u8::min(ycb, self.ppy() - 1)
        } else {
            u8::min(ycb, self.ppy())
        };

        2u32.pow(ycb as u32)
    }
}

struct ParsedTilePart<'a> {
    tile_index: u16,
    tile_part_index: u16,
    component_infos: Vec<ComponentInfo>,
    data: &'a [u8],
}

fn read_tile_part<'a>(
    reader: &mut Reader<'a>,
    main_header: &Header,
    tile_parts: &mut Vec<ParsedTilePart<'a>>,
) -> Result<(), &'static str> {
    if reader.read_marker()? != markers::SOT {
        return Err("expected SOT marker at tile-part start");
    }

    let (mut tile_part_reader, header) = {
        let sot_marker = sot_marker(reader).ok_or("failed to read SOT marker")?;

        let data = if sot_marker.tile_part_length == 0 {
            // Data goes until EOC.
            let data = reader.tail().ok_or("failed to read tile-part payload")?;
            reader.jump_to_end();

            data
        } else {
            // Subtract 12 to account for the marker length.
            let length = (sot_marker.tile_part_length as usize)
                .checked_sub(12)
                .ok_or("tile-part length shorter than header")?;

            let data = reader
                .tail()
                .ok_or("failed to read tile-part payload")?
                .get(..length)
                .ok_or("tile-part payload shorter than declared")?;
            // Skip to the very end in the original reader.
            reader
                .skip_bytes(length)
                .ok_or("failed to advance past tile-part payload")?;

            data
        };

        (Reader::new(data), sot_marker)
    };

    let num_components = main_header.component_infos.len();
    let mut cod = None;
    let mut qcd = None;
    let mut cod_components = vec![None; num_components];
    let mut qcd_components = vec![None; num_components];

    loop {
        match tile_part_reader
            .peek_marker()
            .ok_or("failed to peek tile-part marker")?
        {
            markers::SOD => {
                tile_part_reader.read_marker()?;
                break;
            }
            markers::COD => {
                tile_part_reader.read_marker()?;
                cod = Some(
                    crate::codestream::cod_marker(&mut tile_part_reader)
                        .ok_or("failed to read COD marker")?,
                );
            }
            markers::COC => {
                tile_part_reader.read_marker()?;
                let (component_index, coc) =
                    crate::codestream::coc_marker(&mut tile_part_reader, num_components as u16)
                        .ok_or("failed to read COC marker")?;
                cod_components[component_index as usize] = Some(coc);
            }
            markers::QCD => {
                tile_part_reader.read_marker()?;
                qcd = Some(
                    crate::codestream::qcd_marker(&mut tile_part_reader)
                        .ok_or("failed to read QCD marker")?,
                );
            }
            markers::QCC => {
                tile_part_reader.read_marker()?;
                let (component_index, qcc) =
                    crate::codestream::qcc_marker(&mut tile_part_reader, num_components as u16)
                        .ok_or("failed to read QCC marker")?;
                qcd_components[component_index as usize] = Some(qcc);
            }
            markers::EOC => break,
            _ => {
                tile_part_reader.read_marker()?;
                skip_marker_segment(&mut tile_part_reader)
                    .ok_or("failed to skip a marker during tile part parsing")?;
            }
        }
    }

    // Let's ignore the tile part index and just calculate it ourselves.
    let index = match tile_parts.last() {
        None => 0,
        Some(p) => {
            if p.tile_index != header.tile_index {
                0
            } else {
                p.tile_part_index + 1
            }
        }
    };

    let component_infos = main_header
        .component_infos
        .iter()
        .enumerate()
        .map(|(idx, i)| ComponentInfo {
            size_info: i.size_info,
            coding_style_parameters: cod_components[idx]
                .clone()
                .or_else(|| cod.clone().map(|c| c.component_parameters))
                .unwrap_or(i.coding_style_parameters.clone()),
            quantization_info: qcd_components[idx]
                .clone()
                .or_else(|| qcd.clone())
                .unwrap_or(i.quantization_info.clone()),
        })
        .collect();

    let tile_part = ParsedTilePart {
        data: tile_part_reader
            .tail()
            .ok_or("failed to capture tile-part payload")?,
        tile_index: header.tile_index,
        tile_part_index: index,
        component_infos,
    };

    tile_parts.push(tile_part);

    Ok(())
}

struct TilePartHeader {
    tile_index: u16,
    tile_part_length: u32,
}

/// SOT marker (A.4.2).
fn sot_marker(reader: &mut Reader) -> Option<TilePartHeader> {
    // Length.
    let _ = reader.read_u16()?;

    let tile_index = reader.read_u16()?;
    let tile_part_length = reader.read_u32()?;

    // We infer those ourselves.
    let _tile_part_index = reader.read_byte()? as u16;
    let _num_tile_parts = reader.read_byte()?;

    Some(TilePartHeader {
        tile_index,
        tile_part_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codestream::{
        CodeBlockStyle, CodingStyleFlags, CodingStyleParameters, ComponentCodingStyle,
        ComponentSizeInfo, QuantizationInfo, QuantizationStyle, WaveletTransform,
    };

    /// Test case for the example in B.4.
    #[test]
    fn test_jpeg2000_standard_example_b4() {
        let component_size_info_0 = ComponentSizeInfo {
            precision: 8,
            _is_signed: false,
            horizontal_resolution: 1,
            vertical_resolution: 1,
        };

        let dummy_component_coding_style = ComponentCodingStyle {
            flags: CodingStyleFlags::default(),
            parameters: CodingStyleParameters {
                num_decomposition_levels: 0,
                num_resolution_levels: 0,
                code_block_width: 0,
                code_block_height: 0,
                code_block_style: CodeBlockStyle::default(),
                transformation: WaveletTransform::Irreversible97,
                precinct_exponents: vec![],
            },
        };

        let dummy_quantization_info = QuantizationInfo {
            quantization_style: QuantizationStyle::NoQuantization,
            guard_bits: 0,
            step_sizes: vec![],
        };

        let component_info_0 = ComponentInfo {
            size_info: component_size_info_0,
            coding_style_parameters: dummy_component_coding_style.clone(),
            quantization_info: dummy_quantization_info.clone(),
        };

        let component_size_info_1 = ComponentSizeInfo {
            precision: 8,
            _is_signed: false,
            horizontal_resolution: 2,
            vertical_resolution: 2,
        };

        let component_info_1 = ComponentInfo {
            size_info: component_size_info_1,
            coding_style_parameters: dummy_component_coding_style.clone(),
            quantization_info: dummy_quantization_info.clone(),
        };

        let size_data = SizeData {
            reference_grid_width: 1432,
            reference_grid_height: 954,
            image_area_x_offset: 152,
            image_area_y_offset: 234,
            tile_width: 396,
            tile_height: 297,
            tile_x_offset: 0,
            tile_y_offset: 0,
            component_sizes: vec![component_size_info_0, component_size_info_1],
        };

        assert_eq!(size_data.image_width(), 1280);
        assert_eq!(size_data.image_height(), 720);

        assert_eq!(size_data.num_x_tiles(), 4);
        assert_eq!(size_data.num_y_tiles(), 4);
        assert_eq!(size_data.num_tiles(), 16);

        let tile_0_0 = Tile::new(0, &size_data);
        let coords_0_0 = component_info_0.tile_component_rect(tile_0_0.rect);
        assert_eq!(coords_0_0.x0, 152);
        assert_eq!(coords_0_0.y0, 234);
        assert_eq!(coords_0_0.x1, 396);
        assert_eq!(coords_0_0.y1, 297);
        assert_eq!(coords_0_0.width(), 244);
        assert_eq!(coords_0_0.height(), 63);

        let tile_1_0 = Tile::new(1, &size_data);
        let coords_1_0 = component_info_0.tile_component_rect(tile_1_0.rect);
        assert_eq!(coords_1_0.x0, 396);
        assert_eq!(coords_1_0.y0, 234);
        assert_eq!(coords_1_0.x1, 792);
        assert_eq!(coords_1_0.y1, 297);
        assert_eq!(coords_1_0.width(), 396);
        assert_eq!(coords_1_0.height(), 63);

        let tile_0_1 = Tile::new(4, &size_data);
        let coords_0_1 = component_info_0.tile_component_rect(tile_0_1.rect);
        assert_eq!(coords_0_1.x0, 152);
        assert_eq!(coords_0_1.y0, 297);
        assert_eq!(coords_0_1.x1, 396);
        assert_eq!(coords_0_1.y1, 594);
        assert_eq!(coords_0_1.width(), 244);
        assert_eq!(coords_0_1.height(), 297);

        let tile_1_1 = Tile::new(5, &size_data);
        let coords_1_1 = component_info_0.tile_component_rect(tile_1_1.rect);
        assert_eq!(coords_1_1.x0, 396);
        assert_eq!(coords_1_1.y0, 297);
        assert_eq!(coords_1_1.x1, 792);
        assert_eq!(coords_1_1.y1, 594);
        assert_eq!(coords_1_1.width(), 396);
        assert_eq!(coords_1_1.height(), 297);

        let tile_3_3 = Tile::new(15, &size_data);
        let coords_3_3 = component_info_0.tile_component_rect(tile_3_3.rect);
        assert_eq!(coords_3_3.x0, 1188);
        assert_eq!(coords_3_3.y0, 891);
        assert_eq!(coords_3_3.x1, 1432);
        assert_eq!(coords_3_3.y1, 954);
        assert_eq!(coords_3_3.width(), 244);
        assert_eq!(coords_3_3.height(), 63);

        let tile_0_0_comp1 = component_info_1.tile_component_rect(tile_0_0.rect);
        assert_eq!(tile_0_0_comp1.x0, 76);
        assert_eq!(tile_0_0_comp1.y0, 117);
        assert_eq!(tile_0_0_comp1.x1, 198);
        assert_eq!(tile_0_0_comp1.y1, 149);
        assert_eq!(tile_0_0_comp1.width(), 122);
        assert_eq!(tile_0_0_comp1.height(), 32);

        let tile_1_0_comp1 = component_info_1.tile_component_rect(tile_1_0.rect);
        assert_eq!(tile_1_0_comp1.x0, 198);
        assert_eq!(tile_1_0_comp1.y0, 117);
        assert_eq!(tile_1_0_comp1.x1, 396);
        assert_eq!(tile_1_0_comp1.y1, 149);
        assert_eq!(tile_1_0_comp1.width(), 198);
        assert_eq!(tile_1_0_comp1.height(), 32);

        let tile_0_1_comp1 = component_info_1.tile_component_rect(tile_0_1.rect);
        assert_eq!(tile_0_1_comp1.x0, 76);
        assert_eq!(tile_0_1_comp1.y0, 149);
        assert_eq!(tile_0_1_comp1.x1, 198);
        assert_eq!(tile_0_1_comp1.y1, 297);
        assert_eq!(tile_0_1_comp1.width(), 122);
        assert_eq!(tile_0_1_comp1.height(), 148);

        let tile_1_1_comp1 = component_info_1.tile_component_rect(tile_1_1.rect);
        assert_eq!(tile_1_1_comp1.x0, 198);
        assert_eq!(tile_1_1_comp1.y0, 149);
        assert_eq!(tile_1_1_comp1.x1, 396);
        assert_eq!(tile_1_1_comp1.y1, 297);
        assert_eq!(tile_1_1_comp1.width(), 198);
        assert_eq!(tile_1_1_comp1.height(), 148);

        let tile_2_1 = Tile::new(6, &size_data);
        let tile_2_1_comp1 = component_info_1.tile_component_rect(tile_2_1.rect);
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
