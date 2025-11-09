//! Creating tiles and parsing their constituent tile parts.

use crate::codestream::{
    ComponentInfo, Header, ProgressionOrder, ReaderExt, markers, skip_marker_segment,
};
use crate::decode::SubBandType;
use crate::rect::IntRect;
use hayro_common::byte::Reader;
use log::warn;

/// A single tile in the image.
#[derive(Clone, Debug)]
pub(crate) struct Tile<'a> {
    pub(crate) idx: u32,
    /// The concatenated tile parts that contain all the information for all
    /// constituent codeblocks.
    pub(crate) tile_parts: Vec<&'a [u8]>,
    /// Parameters for each component. In most cases, those are directly
    /// inherited from the main header. But in some cases, individual tiles
    /// might override them.
    // TODO: Don't store size_info
    pub(crate) component_infos: Vec<ComponentInfo>,
    /// The rectangle making up the area of the tile. `x1` and `y1` are
    /// exclusive.
    pub(crate) rect: IntRect,
    pub(crate) progression_order: ProgressionOrder,
    pub(crate) num_layers: u16,
    pub(crate) mct: bool,
}

impl<'a> Tile<'a> {
    fn new(idx: u32, header: &Header) -> Tile<'a> {
        let rect = {
            let size_data = &header.size_data;

            let x_coord = size_data.tile_x_coord(idx);
            let y_coord = size_data.tile_y_coord(idx);

            // See B-7, B-8, B-9 and B-10.
            let x0 = u32::max(
                size_data.tile_x_offset + x_coord * size_data.tile_width,
                size_data.image_area_x_offset,
            );
            let y0 = u32::max(
                size_data.tile_y_offset + y_coord * size_data.tile_height,
                size_data.image_area_y_offset,
            );

            // Note that `x1` and `y1` are exclusive.
            let x1 = u32::min(
                size_data.tile_x_offset + (x_coord + 1) * size_data.tile_width,
                size_data.reference_grid_width,
            );
            let y1 = u32::min(
                size_data.tile_y_offset + (y_coord + 1) * size_data.tile_height,
                size_data.reference_grid_height,
            );

            IntRect::from_ltrb(x0, y0, x1, y1)
        };

        Tile {
            idx,
            // Will be filled once we start parsing.
            tile_parts: vec![],
            rect,
            // By default, each tile inherits the settings from the main
            // header. When parsing the tile parts, some of these settings
            // might be overridden.
            component_infos: header.component_infos.clone(),
            progression_order: header.global_coding_style.progression_order,
            mct: header.global_coding_style.mct,
            num_layers: header.global_coding_style.num_layers,
        }
    }

    /// Return an iterator over the component tiles.
    pub(crate) fn component_tiles(&self) -> impl Iterator<Item = ComponentTile<'_>> {
        self.component_infos
            .iter()
            .map(|i| ComponentTile::new(self, i))
    }
}

/// Create the tiles and parse their constituent tile parts.
pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    main_header: &'a Header,
) -> Result<Vec<Tile<'a>>, &'static str> {
    let mut tiles = (0..main_header.size_data.num_tiles() as usize)
        .map(|idx| Tile::new(idx as u32, main_header))
        .collect::<Vec<_>>();

    parse_tile_part(reader, main_header, &mut tiles, true)?;

    while reader.peek_marker() == Some(markers::SOT) {
        parse_tile_part(reader, main_header, &mut tiles, false)?;
    }

    // TODO: Add this for strict mode.
    // if reader.read_marker()? != markers::EOC {
    //     return Err("expected EOC marker when parsing tiles");
    // }

    Ok(tiles)
}

fn parse_tile_part<'a>(
    reader: &mut Reader<'a>,
    main_header: &Header,
    tiles: &mut [Tile<'a>],
    first: bool,
) -> Result<(), &'static str> {
    if reader.read_marker()? != markers::SOT {
        return Err("expected SOT marker at tile-part start");
    }

    let tile_part_header = sot_marker(reader).ok_or("failed to read SOT marker")?;

    if tile_part_header.tile_index as u32 >= main_header.size_data.num_tiles() {
        return Err("invalid tile index in tile-part header");
    }

    let data_len = if tile_part_header.tile_part_length == 0 {
        reader.tail().map(|d| d.len()).unwrap_or(0)
    } else {
        // Subtract 12 to account for the marker length.

        (tile_part_header.tile_part_length as usize)
            .checked_sub(12)
            .ok_or("tile-part length shorter than header")?
    };

    let start = reader.offset();

    let tile = &mut tiles[tile_part_header.tile_index as usize];
    let num_components = tile.component_infos.len();

    loop {
        let Some(marker) = reader.peek_marker() else {
            warn!(
                "expected marker in tile-part, but didn't find one. tile \
            part will be ignored."
            );

            return Ok(());
        };

        match marker {
            markers::SOD => {
                reader.read_marker()?;
                break;
            }
            // COD, COC, QCD and QCC should only be used in the _first_
            // tile-part header, if they appear at all.
            markers::COD => {
                reader.read_marker()?;
                let cod =
                    crate::codestream::cod_marker(reader).ok_or("failed to read COD marker")?;

                if first {
                    tile.mct = cod.mct;
                    tile.num_layers = cod.num_layers;
                    tile.progression_order = cod.progression_order;

                    for component in &mut tile.component_infos {
                        component.coding_style = cod.component_parameters.clone();
                    }
                } else {
                    warn!("encountered unexpected COD marker in tile-part header");
                }
            }
            markers::COC => {
                reader.read_marker()?;

                let (component_index, coc) =
                    crate::codestream::coc_marker(reader, num_components as u16)
                        .ok_or("failed to read COC marker")?;

                if first {
                    tile.component_infos
                        .get_mut(component_index as usize)
                        .ok_or("invalid component index in tile-part header")?
                        .coding_style = coc;
                } else {
                    warn!("encountered unexpected COC marker in tile-part header");
                }
            }
            markers::QCD => {
                reader.read_marker()?;
                let qcd =
                    crate::codestream::qcd_marker(reader).ok_or("failed to read QCD marker")?;

                if first {
                    for component_info in &mut tile.component_infos {
                        component_info.quantization_info = qcd.clone();
                    }
                } else {
                    warn!("encountered unexpected QCD marker in tile-part header");
                }
            }
            markers::QCC => {
                reader.read_marker()?;
                let (component_index, qcc) =
                    crate::codestream::qcc_marker(reader, num_components as u16)
                        .ok_or("failed to read QCC marker")?;

                if first {
                    tile.component_infos
                        .get_mut(component_index as usize)
                        .ok_or("invalid component index in tile-part header")?
                        .quantization_info = qcc.clone();
                } else {
                    warn!("encountered unexpected QCC marker in tile-part header");
                }
            }
            markers::EOC => break,
            _ => {
                reader.read_marker()?;
                skip_marker_segment(reader)
                    .ok_or("failed to skip a marker during tile part parsing")?;
            }
        }
    }

    for ci in &tile.component_infos {
        if ci
            .coding_style
            .parameters
            .code_block_style
            .selective_arithmetic_coding_bypass
        {
            return Err("unsupported code-block style features encountered during decoding");
        }
    }

    let remaining_bytes = if let Some(len) = data_len.checked_sub(reader.offset() - start) {
        len
    } else {
        warn!("didn't find sufficient data in tile part");

        return Ok(());
    };

    tile.tile_parts.push(
        reader
            .read_bytes(remaining_bytes)
            .ok_or("failed to get tile part data")?,
    );

    Ok(())
}

/// A tile, instantiated to a specific component.
#[derive(Debug, Copy, Clone)]
pub(crate) struct ComponentTile<'a> {
    /// The underlying tile.
    pub(crate) tile: &'a Tile<'a>,
    /// The information of the component of the tile.
    pub(crate) component_info: &'a ComponentInfo,
    /// The rectangle of the component tile.
    pub(crate) rect: IntRect,
}

impl<'a> ComponentTile<'a> {
    pub(crate) fn new(tile: &'a Tile<'a>, component_info: &'a ComponentInfo) -> Self {
        let tile_rect = tile.rect;

        let rect = if component_info.size_info.horizontal_resolution == 1
            && component_info.size_info.vertical_resolution == 1
        {
            tile_rect
        } else {
            // As described in B-12.
            let t_x0 = tile_rect
                .x0
                .div_ceil(component_info.size_info.horizontal_resolution as u32);
            let t_y0 = tile_rect
                .y0
                .div_ceil(component_info.size_info.vertical_resolution as u32);
            let t_x1 = tile_rect
                .x1
                .div_ceil(component_info.size_info.horizontal_resolution as u32);
            let t_y1 = tile_rect
                .y1
                .div_ceil(component_info.size_info.vertical_resolution as u32);

            IntRect::from_ltrb(t_x0, t_y0, t_x1, t_y1)
        };

        ComponentTile {
            tile,
            component_info,
            rect,
        }
    }

    pub(crate) fn resolution_tiles(&self) -> impl IntoIterator<Item = ResolutionTile<'_>> {
        (0..self
            .component_info
            .coding_style
            .parameters
            .num_resolution_levels)
            .map(|r| ResolutionTile::new(*self, r))
    }
}

/// A tile instantiated to a specific resolution of a component tile.
pub(crate) struct ResolutionTile<'a> {
    /// The resolution of the tile.
    pub(crate) resolution: u16,
    /// The decomposition level of the tile.
    pub(crate) decomposition_level: u16,
    /// The underlying component tile.
    pub(crate) component_tile: ComponentTile<'a>,
    /// The rectangle of the resolution tile.
    pub(crate) rect: IntRect,
}

impl<'a> ResolutionTile<'a> {
    pub(crate) fn new(component_tile: ComponentTile, resolution: u16) -> ResolutionTile {
        assert!(
            component_tile
                .component_info
                .coding_style
                .parameters
                .num_resolution_levels
                > resolution
        );

        let rect = {
            // See formula B-14.
            let n_l = component_tile
                .component_info
                .coding_style
                .parameters
                .num_decomposition_levels;

            let tx0 = component_tile
                .rect
                .x0
                .div_ceil(2u32.pow(n_l as u32 - resolution as u32));
            let ty0 = component_tile
                .rect
                .y0
                .div_ceil(2u32.pow(n_l as u32 - resolution as u32));
            let tx1 = component_tile
                .rect
                .x1
                .div_ceil(2u32.pow(n_l as u32 - resolution as u32));
            let ty1 = component_tile
                .rect
                .y1
                .div_ceil(2u32.pow(n_l as u32 - resolution as u32));

            IntRect::from_ltrb(tx0, ty0, tx1, ty1)
        };

        // Decomposition level and resolution level are inversely related
        // to each other. In addition to that, there is always one more
        // resolution than decomposition levels (resolution level 0 only
        // include the LL subband of the N_L decomposition, resolution level
        // 1 includes the HL, LH and HH subbands of the N_L decomposition.
        let decomposition_level = {
            if resolution == 0 {
                component_tile
                    .component_info
                    .coding_style
                    .parameters
                    .num_decomposition_levels
            } else {
                component_tile
                    .component_info
                    .coding_style
                    .parameters
                    .num_decomposition_levels
                    - (resolution - 1)
            }
        };

        ResolutionTile {
            resolution,
            decomposition_level,
            component_tile,
            rect,
        }
    }

    pub(crate) fn sub_band_rect(&self, sub_band_type: SubBandType) -> IntRect {
        // This is the only permissible sub-band type for the given resolution.
        if self.resolution == 0 {
            assert_eq!(sub_band_type, SubBandType::LowLow);
        }

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

        let numerator_x = 2u32.pow(self.decomposition_level as u32 - 1) * xo_b;
        let numerator_y = 2u32.pow(self.decomposition_level as u32 - 1) * yo_b;
        let denominator = 2u32.pow(self.decomposition_level as u32);

        let tbx_0 = self
            .component_tile
            .rect
            .x0
            .saturating_sub(numerator_x)
            .div_ceil(denominator);
        let tbx_1 = self
            .component_tile
            .rect
            .x1
            .saturating_sub(numerator_x)
            .div_ceil(denominator);
        let tby_0 = self
            .component_tile
            .rect
            .y0
            .saturating_sub(numerator_y)
            .div_ceil(denominator);
        let tby_1 = self
            .component_tile
            .rect
            .y1
            .saturating_sub(numerator_y)
            .div_ceil(denominator);

        IntRect::from_ltrb(tbx_0, tby_0, tbx_1, tby_1)
    }

    /// The exponent for determining the horizontal size of a precinct.
    ///
    /// `PPx` in the specification.
    pub(crate) fn precinct_exponent_x(&self) -> u8 {
        self.component_tile
            .component_info
            .coding_style
            .parameters
            .precinct_exponents[self.resolution as usize]
            .0
    }

    /// The exponent for determining the vertical size of a precinct.
    ///
    /// `PPx` in the specification.
    pub(crate) fn precinct_exponent_y(&self) -> u8 {
        self.component_tile
            .component_info
            .coding_style
            .parameters
            .precinct_exponents[self.resolution as usize]
            .1
    }

    pub(crate) fn num_precincts_x(&self) -> u32 {
        // See B-16.
        let IntRect { x0, x1, .. } = self.rect;

        if x0 == x1 {
            0
        } else {
            x1.div_ceil(2u32.pow(self.precinct_exponent_x() as u32))
                - x0 / 2u32.pow(self.precinct_exponent_x() as u32)
        }
    }

    pub(crate) fn num_precincts_y(&self) -> u32 {
        // See B-16.
        let IntRect { y0, y1, .. } = self.rect;

        if y0 == y1 {
            0
        } else {
            y1.div_ceil(2u32.pow(self.precinct_exponent_y() as u32))
                - y0 / 2u32.pow(self.precinct_exponent_y() as u32)
        }
    }

    pub(crate) fn num_precincts(&self) -> u32 {
        self.num_precincts_x() * self.num_precincts_y()
    }

    pub(crate) fn code_block_width(&self) -> u32 {
        // See B-17.
        let xcb = self
            .component_tile
            .component_info
            .coding_style
            .parameters
            .code_block_width;

        let xcb = if self.resolution > 0 {
            u8::min(xcb, self.precinct_exponent_x() - 1)
        } else {
            u8::min(xcb, self.precinct_exponent_x())
        };

        2u32.pow(xcb as u32)
    }

    pub(crate) fn code_block_height(&self) -> u32 {
        // See B-18.
        let ycb = self
            .component_tile
            .component_info
            .coding_style
            .parameters
            .code_block_height;

        let ycb = if self.resolution > 0 {
            u8::min(ycb, self.precinct_exponent_y() - 1)
        } else {
            u8::min(ycb, self.precinct_exponent_y())
        };

        2u32.pow(ycb as u32)
    }
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
        CodeBlockStyle, CodingStyleComponent, CodingStyleDefault, CodingStyleFlags,
        CodingStyleParameters, ComponentSizeInfo, QuantizationInfo, QuantizationStyle, SizeData,
        WaveletTransform,
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

        let dummy_component_coding_style = CodingStyleComponent {
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
            coding_style: dummy_component_coding_style.clone(),
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
            coding_style: dummy_component_coding_style.clone(),
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

        let header = Header {
            size_data,
            // Just dummy values.
            global_coding_style: CodingStyleDefault {
                progression_order: ProgressionOrder::LayerResolutionComponentPosition,
                num_layers: 0,
                mct: false,
                component_parameters: CodingStyleComponent {
                    flags: Default::default(),
                    parameters: CodingStyleParameters {
                        num_decomposition_levels: 0,
                        num_resolution_levels: 0,
                        code_block_width: 0,
                        code_block_height: 0,
                        code_block_style: Default::default(),
                        transformation: WaveletTransform::Irreversible97,
                        precinct_exponents: vec![],
                    },
                },
            },
            component_infos: vec![],
        };

        let tile_0_0 = Tile::new(0, &header);
        let coords_0_0 = ComponentTile::new(&tile_0_0, &component_info_0).rect;
        assert_eq!(coords_0_0.x0, 152);
        assert_eq!(coords_0_0.y0, 234);
        assert_eq!(coords_0_0.x1, 396);
        assert_eq!(coords_0_0.y1, 297);
        assert_eq!(coords_0_0.width(), 244);
        assert_eq!(coords_0_0.height(), 63);

        let tile_1_0 = Tile::new(1, &header);
        let coords_1_0 = ComponentTile::new(&tile_1_0, &component_info_0).rect;
        assert_eq!(coords_1_0.x0, 396);
        assert_eq!(coords_1_0.y0, 234);
        assert_eq!(coords_1_0.x1, 792);
        assert_eq!(coords_1_0.y1, 297);
        assert_eq!(coords_1_0.width(), 396);
        assert_eq!(coords_1_0.height(), 63);

        let tile_0_1 = Tile::new(4, &header);
        let coords_0_1 = ComponentTile::new(&tile_0_1, &component_info_0).rect;
        assert_eq!(coords_0_1.x0, 152);
        assert_eq!(coords_0_1.y0, 297);
        assert_eq!(coords_0_1.x1, 396);
        assert_eq!(coords_0_1.y1, 594);
        assert_eq!(coords_0_1.width(), 244);
        assert_eq!(coords_0_1.height(), 297);

        let tile_1_1 = Tile::new(5, &header);
        let coords_1_1 = ComponentTile::new(&tile_1_1, &component_info_0).rect;
        assert_eq!(coords_1_1.x0, 396);
        assert_eq!(coords_1_1.y0, 297);
        assert_eq!(coords_1_1.x1, 792);
        assert_eq!(coords_1_1.y1, 594);
        assert_eq!(coords_1_1.width(), 396);
        assert_eq!(coords_1_1.height(), 297);

        let tile_3_3 = Tile::new(15, &header);
        let coords_3_3 = ComponentTile::new(&tile_3_3, &component_info_0).rect;
        assert_eq!(coords_3_3.x0, 1188);
        assert_eq!(coords_3_3.y0, 891);
        assert_eq!(coords_3_3.x1, 1432);
        assert_eq!(coords_3_3.y1, 954);
        assert_eq!(coords_3_3.width(), 244);
        assert_eq!(coords_3_3.height(), 63);

        let tile_0_0_comp1 = ComponentTile::new(&tile_0_0, &component_info_1).rect;
        assert_eq!(tile_0_0_comp1.x0, 76);
        assert_eq!(tile_0_0_comp1.y0, 117);
        assert_eq!(tile_0_0_comp1.x1, 198);
        assert_eq!(tile_0_0_comp1.y1, 149);
        assert_eq!(tile_0_0_comp1.width(), 122);
        assert_eq!(tile_0_0_comp1.height(), 32);

        let tile_1_0_comp1 = ComponentTile::new(&tile_1_0, &component_info_1).rect;
        assert_eq!(tile_1_0_comp1.x0, 198);
        assert_eq!(tile_1_0_comp1.y0, 117);
        assert_eq!(tile_1_0_comp1.x1, 396);
        assert_eq!(tile_1_0_comp1.y1, 149);
        assert_eq!(tile_1_0_comp1.width(), 198);
        assert_eq!(tile_1_0_comp1.height(), 32);

        let tile_0_1_comp1 = ComponentTile::new(&tile_0_1, &component_info_1).rect;
        assert_eq!(tile_0_1_comp1.x0, 76);
        assert_eq!(tile_0_1_comp1.y0, 149);
        assert_eq!(tile_0_1_comp1.x1, 198);
        assert_eq!(tile_0_1_comp1.y1, 297);
        assert_eq!(tile_0_1_comp1.width(), 122);
        assert_eq!(tile_0_1_comp1.height(), 148);

        let tile_1_1_comp1 = ComponentTile::new(&tile_1_1, &component_info_1).rect;
        assert_eq!(tile_1_1_comp1.x0, 198);
        assert_eq!(tile_1_1_comp1.y0, 149);
        assert_eq!(tile_1_1_comp1.x1, 396);
        assert_eq!(tile_1_1_comp1.y1, 297);
        assert_eq!(tile_1_1_comp1.width(), 198);
        assert_eq!(tile_1_1_comp1.height(), 148);

        let tile_2_1 = Tile::new(6, &header);
        let tile_2_1_comp1 = ComponentTile::new(&tile_2_1, &component_info_1).rect;
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
