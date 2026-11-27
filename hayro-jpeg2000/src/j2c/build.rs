use super::decode::{DecompositionStorage, TileDecompositions};
use super::rect::IntRect;
use super::tag_tree::TagTree;
use super::tile::{ResolutionTile, Tile};
use crate::cancel::Cancel;
use crate::error::{DecodingError, Result, ValidationError};
use alloc::vec;
use core::iter;
use core::ops::Range;

pub(crate) fn build(
    tile: &Tile<'_>,
    storage: &mut DecompositionStorage<'_>,
    skipped_resolution_levels: u8,
    cancel: Cancel<'_>,
) -> Result<()> {
    build_decompositions(tile, storage, skipped_resolution_levels, cancel)
}

fn build_decompositions(
    tile: &Tile<'_>,
    storage: &mut DecompositionStorage<'_>,
    skipped_resolution_levels: u8,
    cancel: Cancel<'_>,
) -> Result<()> {
    let mut component_samples = vec![];

    for component_tile in tile.component_tiles() {
        // Tile-part headers can lower the resolution levels below the skipped count.
        let decoded_resolutions = component_tile
            .component_info
            .num_resolution_levels()
            .checked_sub(skipped_resolution_levels)
            .filter(|r| *r > 0)
            .ok_or(ValidationError::InvalidComponentMetadata)?;
        let top_resolution = ResolutionTile::new(component_tile, decoded_resolutions - 1);
        component_samples.push(
            usize::try_from(top_resolution.rect.area())
                .map_err(|_| ValidationError::ImageTooLarge)?,
        );
    }
    let max_coefficients = component_samples.iter().copied().max().unwrap_or(0);

    if storage.coefficients.is_empty() {
        // Fast path that requests pre-zeroed memory from the OS where available.
        storage.coefficients = vec![0.0; max_coefficients];
    } else {
        cancel.resize(&mut storage.coefficients, max_coefficients)?;
    }

    for (component_idx, component_tile) in tile.component_tiles().enumerate() {
        let mut coefficient_counter = 0_usize;
        let decoded_resolutions =
            component_tile.component_info.num_resolution_levels() - skipped_resolution_levels;
        let d_start = storage.decompositions.len();
        let mut resolution_tiles = component_tile.resolution_tiles();

        let mut build_sub_band = |sub_band_type: SubBandType,
                                  resolution_tile: &ResolutionTile<'_>,
                                  storage: &mut DecompositionStorage<'_>|
         -> Result<usize> {
            let sub_band_rect = resolution_tile.sub_band_rect(sub_band_type);

            let precincts = build_precincts(resolution_tile, sub_band_rect, tile, storage, cancel)?;

            let added_coefficients = if resolution_tile.resolution < decoded_resolutions {
                usize::try_from(sub_band_rect.area()).map_err(|_| ValidationError::ImageTooLarge)?
            } else {
                0
            };
            let next_coefficient_counter = coefficient_counter
                .checked_add(added_coefficients)
                .ok_or(ValidationError::ImageTooLarge)?;
            let coefficients = coefficient_counter..next_coefficient_counter;
            coefficient_counter = next_coefficient_counter;

            let idx = storage.sub_bands.len();
            storage.sub_bands.push(SubBand {
                sub_band_type,
                rect: sub_band_rect,
                precincts: precincts.clone(),
                coefficients,
            });

            Ok(idx)
        };

        let ll_resolution_tile = resolution_tiles.next().unwrap();
        let first_ll_sub_band = build_sub_band(SubBandType::LowLow, &ll_resolution_tile, storage)?;

        for resolution_tile in resolution_tiles {
            let decomposition = Decomposition {
                sub_bands: [
                    build_sub_band(SubBandType::HighLow, &resolution_tile, storage)?,
                    build_sub_band(SubBandType::LowHigh, &resolution_tile, storage)?,
                    build_sub_band(SubBandType::HighHigh, &resolution_tile, storage)?,
                ],
                rect: resolution_tile.rect,
            };

            storage.decompositions.push(decomposition);
        }

        let d_end = storage.decompositions.len();

        storage.tile_decompositions.push(TileDecompositions {
            decompositions: d_start..d_end,
            first_ll_sub_band,
            coefficient_count: coefficient_counter,
        });

        assert_eq!(coefficient_counter, component_samples[component_idx]);
    }

    Ok(())
}

fn build_precincts(
    resolution_tile: &ResolutionTile<'_>,
    sub_band_rect: IntRect,
    tile: &Tile<'_>,
    storage: &mut DecompositionStorage<'_>,
    cancel: Cancel<'_>,
) -> Result<Range<usize>> {
    let start = storage.precincts.len();

    for precinct_data in resolution_tile
        .precincts()
        .ok_or(DecodingError::InvalidPrecinct)?
    {
        cancel.check()?;

        let rect = precinct_data.rect.intersect(sub_band_rect);
        let columns = block_indices(rect.x0, rect.x1, resolution_tile.code_block_width());
        let rows = block_indices(rect.y0, rect.y1, resolution_tile.code_block_height());
        let code_blocks_x = columns.end - columns.start;
        let code_blocks_y = rows.end - rows.start;

        let blocks = build_code_blocks(
            columns,
            rows,
            sub_band_rect,
            resolution_tile,
            tile,
            storage,
            cancel,
        )?;

        let code_inclusion_tree =
            TagTree::new(code_blocks_x, code_blocks_y, &mut storage.tag_tree_nodes);
        let zero_bitplane_tree =
            TagTree::new(code_blocks_x, code_blocks_y, &mut storage.tag_tree_nodes);

        storage.precincts.push(Precinct {
            code_blocks: blocks,
            code_inclusion_tree,
            zero_bitplane_tree,
        });
    }

    let end = storage.precincts.len();

    Ok(start..end)
}

fn build_code_blocks(
    columns: Range<u32>,
    rows: Range<u32>,
    sub_band_rect: IntRect,
    tile_instance: &ResolutionTile<'_>,
    tile: &Tile<'_>,
    storage: &mut DecompositionStorage<'_>,
    cancel: Cancel<'_>,
) -> Result<Range<usize>> {
    let code_block_width = tile_instance.code_block_width();
    let code_block_height = tile_instance.code_block_height();

    let start = storage.code_blocks.len();

    for row in rows.clone() {
        cancel.check()?;

        let y = row * code_block_height;

        for column in columns.clone() {
            let x = column * code_block_width;
            let area = IntRect::from_xywh(x, y, code_block_width, code_block_height)
                .intersect(sub_band_rect);

            let start = storage.layers.len();
            storage.layers.extend(iter::repeat_n(
                Layer { segments: None },
                tile.num_layers as usize,
            ));
            let end = storage.layers.len();

            storage.code_blocks.push(CodeBlock {
                x_idx: column - columns.start,
                y_idx: row - rows.start,
                rect: area,
                has_been_included: false,
                missing_bit_planes: 0,
                l_block: 3,
                number_of_coding_passes: 0,
                layers: start..end,
                non_empty_layer_count: 0,
            });
        }
    }

    let end = storage.code_blocks.len();

    Ok(start..end)
}

fn block_indices(start: u32, end: u32, size: u32) -> Range<u32> {
    if start == end {
        0..0
    } else {
        start / size..end.div_ceil(size)
    }
}

pub(crate) struct Decomposition {
    // In the order low-high, high-low and high-high.
    pub(crate) sub_bands: [usize; 3],
    pub(crate) rect: IntRect,
}

#[derive(Clone)]
pub(crate) struct SubBand {
    pub(crate) sub_band_type: SubBandType,
    pub(crate) rect: IntRect,
    pub(crate) precincts: Range<usize>,
    pub(crate) coefficients: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubBandType {
    LowLow = 0,
    HighLow = 1,
    LowHigh = 2,
    HighHigh = 3,
}

#[derive(Clone)]
pub(crate) struct Precinct {
    pub(crate) code_blocks: Range<usize>,
    pub(crate) code_inclusion_tree: TagTree,
    pub(crate) zero_bitplane_tree: TagTree,
}

pub(crate) struct PrecinctData {
    pub(crate) r_x: u32,
    pub(crate) r_y: u32,
    pub(crate) rect: IntRect,
    pub(crate) idx: u64,
}

#[derive(Clone)]
pub(crate) struct CodeBlock {
    pub(crate) rect: IntRect,
    pub(crate) x_idx: u32,
    pub(crate) y_idx: u32,
    pub(crate) layers: Range<usize>,
    pub(crate) has_been_included: bool,
    pub(crate) missing_bit_planes: u8,
    pub(crate) number_of_coding_passes: u8,
    pub(crate) l_block: u32,
    pub(crate) non_empty_layer_count: u8,
}

pub(crate) struct Segment<'a> {
    pub(crate) idx: u8,
    pub(crate) coding_pases: u8,
    pub(crate) data_length: u32,
    pub(crate) data: &'a [u8],
}

#[derive(Clone)]
pub(crate) struct Layer {
    pub(crate) segments: Option<Range<usize>>,
}
