//! Progression iterators, defined in Section B.12.
//!
//! A progression iterator essentially yields tuples of
//! (layer_num, resolution, component, precinct) in a specific order that
//! determines in which order the data appears in the codestream.

use crate::tile::{ComponentTile, ResolutionTile, Tile};
use std::iter;

// TODO: Refactor this whole module.

#[derive(Default, Copy, Clone, Debug, PartialEq)]
pub(crate) struct ProgressionData {
    pub(crate) layer_num: u16,
    pub(crate) resolution: u16,
    pub(crate) component: u8,
    pub(crate) precinct: u32,
}

pub(crate) struct IteratorInput<'a> {
    layers: u16,
    tile: &'a Tile<'a>,
    max_resolutions: u16,
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(tile: &'a Tile<'a>) -> Self {
        let max_resolutions = tile
            .component_infos
            .iter()
            .map(|c| c.coding_style.parameters.num_resolution_levels)
            .max()
            .unwrap_or(0);

        Self {
            layers: tile.num_layers,
            tile,
            max_resolutions,
        }
    }

    fn component_tiles(&'a self) -> Vec<ComponentTile<'a>> {
        self.tile
            .component_infos
            .iter()
            .map(|c| ComponentTile::new(self.tile, c))
            .collect::<Vec<_>>()
    }
}

/// B.12.1.1 Layer-resolution level-component-position progression.
pub(crate) fn layer_resolution_component_position_progression(
    input: &IteratorInput<'_>,
) -> impl Iterator<Item = ProgressionData> {
    let num_components = input.tile.component_infos.len();

    let component_tiles = input.component_tiles();

    let mut layer = 0;
    let mut resolution = 0;
    let mut component_idx = 0;
    let mut resolution_tile = ResolutionTile::new(component_tiles[0], resolution);
    let mut precinct = 0;

    iter::from_fn(move || {
        if precinct == resolution_tile.num_precincts() {
            loop {
                precinct = 0;
                component_idx += 1;

                if component_idx == num_components {
                    component_idx = 0;

                    resolution += 1;

                    if resolution == input.max_resolutions {
                        resolution = 0;
                        layer += 1;

                        if layer == input.layers {
                            return None;
                        }
                    }
                }

                resolution_tile = ResolutionTile::new(component_tiles[component_idx], resolution);

                // Only yield if the resolution tile has precincts, otherwise
                // we need to keep advancing.
                if resolution_tile.num_precincts() != 0 {
                    break;
                }
            }
        }

        let data = ProgressionData {
            layer_num: layer,
            resolution,
            component: component_idx as u8,
            precinct,
        };

        precinct += 1;

        Some(data)
    })
}

/// B.12.1.2 Resolution level-layer-component-position progression.
pub(crate) fn resolution_layer_component_position_progression(
    input: &IteratorInput<'_>,
) -> impl Iterator<Item = ProgressionData> {
    let num_components = input.tile.component_infos.len();

    let component_tiles = input.component_tiles();

    let mut layer = 0;
    let mut resolution = 0;
    let mut component_idx = 0;
    let mut resolution_tile = ResolutionTile::new(component_tiles[component_idx], resolution);
    let mut precinct = 0;

    iter::from_fn(move || {
        if resolution == input.max_resolutions {
            return None;
        }

        if precinct == resolution_tile.num_precincts() {
            loop {
                precinct = 0;
                component_idx += 1;

                if component_idx == num_components {
                    component_idx = 0;
                    layer += 1;

                    if layer == input.layers {
                        layer = 0;
                        resolution += 1;

                        if resolution == input.max_resolutions {
                            return None;
                        }
                    }
                }

                resolution_tile = ResolutionTile::new(component_tiles[component_idx], resolution);

                // Only yield if the resolution tile has precincts, otherwise
                // we need to keep advancing.
                if resolution_tile.num_precincts() != 0 {
                    break;
                }
            }
        }

        let data = ProgressionData {
            layer_num: layer,
            resolution,
            component: component_idx as u8,
            precinct,
        };

        precinct += 1;

        Some(data)
    })
}

pub(crate) fn build_resolution_position_component_layer_sequence(
    input: &IteratorInput<'_>,
) -> impl Iterator<Item = ProgressionData> {
    let mut sequence_b = Vec::new();
    let num_components = input.tile.component_infos.len();
    let component_tiles = input.component_tiles();

    for resolution in 0..input.max_resolutions {
        // Currently, we are assuming that each component resolution tile
        // has the same number of precincts and that they have the same
        // resolution. TODO: Add debug assertion.
        let component_tile = component_tiles[0];
        let resolution_tile = ResolutionTile::new(component_tile, resolution);
        let num_precincts = resolution_tile.num_precincts();

        for precinct in 0..num_precincts {
            for component_idx in 0..num_components {
                for layer in 0..input.layers {
                    sequence_b.push(ProgressionData {
                        layer_num: layer,
                        resolution,
                        component: component_idx as u8,
                        precinct,
                    });
                }
            }
        }
    }

    sequence_b.into_iter()
}

pub(crate) fn build_position_component_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> impl Iterator<Item = ProgressionData> {
    // Note that the order of fields here is important!
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct PrecinctStore {
        precinct_y: u32,
        precinct_x: u32,
        component_idx: u8,
        resolution: u16,
        precinct_idx: u32,
    }

    let mut elements = vec![];

    for (component_idx, component) in input.tile.component_tiles().enumerate() {
        for (resolution, resolution_tile) in component.resolution_tiles().enumerate() {
            elements.extend(resolution_tile.precincts().map(|d| PrecinctStore {
                precinct_y: d.rect.y0,
                precinct_x: d.rect.x0,
                component_idx: component_idx as u8,
                resolution: resolution as u16,
                precinct_idx: d.idx,
            }))
        }
    }

    elements.sort();

    elements.into_iter().flat_map(|e| {
        (0..input.layers).map(move |layer| ProgressionData {
            layer_num: layer,
            resolution: e.resolution,
            component: e.component_idx,
            precinct: e.precinct_idx,
        })
    })
}

pub(crate) fn build_component_position_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let tile_rect = input.tile.rect;

    for (component_idx, component_tile) in input.tile.component_tiles().enumerate() {
        let num_resolution_levels = component_tile
            .component_info
            .coding_style
            .parameters
            .num_resolution_levels;

        for y in tile_rect.y0..tile_rect.y1 {
            for x in tile_rect.x0..tile_rect.x1 {
                for resolution in 0..num_resolution_levels {
                    let resolution_tile = ResolutionTile::new(component_tile, resolution);

                    if let Some(precinct) = find_precinct_index(&resolution_tile, x, y) {
                        for layer in 0..input.layers {
                            sequence.push(ProgressionData {
                                layer_num: layer,
                                resolution,
                                component: component_idx as u8,
                                precinct,
                            });
                        }
                    }
                }
            }
        }
    }

    let new = {
        // Note that the order of fields here is important!
        #[derive(PartialEq, Eq, PartialOrd, Ord)]
        struct PrecinctStore {
            component_idx: u8,
            precinct_y: u32,
            precinct_x: u32,
            resolution: u16,
            precinct_idx: u32,
        }

        let mut elements = vec![];

        for (component_idx, component) in input.tile.component_tiles().enumerate() {
            for (resolution, resolution_tile) in component.resolution_tiles().enumerate() {
                elements.extend(resolution_tile.precincts().map(|d| PrecinctStore {
                    precinct_y: d.r_y,
                    precinct_x: d.r_x,
                    component_idx: component_idx as u8,
                    resolution: resolution as u16,
                    precinct_idx: d.idx,
                }))
            }
        }

        elements.sort();

        elements
            .into_iter()
            .flat_map(|e| {
                (0..input.layers).map(move |layer| ProgressionData {
                    layer_num: layer,
                    resolution: e.resolution,
                    component: e.component_idx,
                    precinct: e.precinct_idx,
                })
            })
            .collect()
    };

    assert_eq!(sequence, new);

    new
}

fn find_precinct_index(resolution_tile: &ResolutionTile, x: u32, y: u32) -> Option<u32> {
    if resolution_tile.num_precincts() == 0 {
        return None;
    }

    let component_info = resolution_tile.component_tile.component_info;
    let tile_rect = resolution_tile.component_tile.tile.rect;

    let num_decomposition_levels = component_info
        .coding_style
        .parameters
        .num_decomposition_levels as u32;
    let resolution = resolution_tile.resolution as u32;
    if resolution > num_decomposition_levels {
        return None;
    }

    let vertical_resolution = component_info.size_info.vertical_resolution as u32;
    let horizontal_resolution = component_info.size_info.horizontal_resolution as u32;
    if vertical_resolution == 0 || horizontal_resolution == 0 {
        return None;
    }

    let base_shift = num_decomposition_levels.checked_sub(resolution)?;
    let resolution_scale = 1u64 << base_shift;

    let y_stride_shift = resolution_tile.precinct_exponent_y() as u32 + base_shift;
    let x_stride_shift = resolution_tile.precinct_exponent_x() as u32 + base_shift;
    let y_stride_factor = 1u64 << y_stride_shift;
    let x_stride_factor = 1u64 << x_stride_shift;

    let y_stride = vertical_resolution as u64 * y_stride_factor;
    let x_stride = horizontal_resolution as u64 * x_stride_factor;
    if y_stride == 0 || x_stride == 0 {
        return None;
    }

    let y_val = y as u64;
    let x_val = x as u64;
    let ty0 = tile_rect.y0 as u64;
    let tx0 = tile_rect.x0 as u64;
    let try0 = resolution_tile.rect.y0 as u64;
    let trx0 = resolution_tile.rect.x0 as u64;

    let cond1 = y_val.is_multiple_of(y_stride);
    let cond2 = y_val == ty0 && !(try0 * resolution_scale).is_multiple_of(y_stride);
    if !(cond1 || cond2) {
        return None;
    }

    let cond3 = x_val.is_multiple_of(x_stride);
    let cond4 = x_val == tx0 && !(trx0 * resolution_scale).is_multiple_of(x_stride);
    if !(cond3 || cond4) {
        return None;
    }

    let horizontal_denom = horizontal_resolution as u64 * resolution_scale;
    let vertical_denom = vertical_resolution as u64 * resolution_scale;
    if horizontal_denom == 0 || vertical_denom == 0 {
        return None;
    }

    let precinct_x_scale = 1u64 << (resolution_tile.precinct_exponent_x() as u32);
    let precinct_y_scale = 1u64 << (resolution_tile.precinct_exponent_y() as u32);

    let p1 = x_val.div_ceil(horizontal_denom) / precinct_x_scale;
    let p2 = trx0 / precinct_x_scale;
    let diff_x = p1.checked_sub(p2)?;

    let p4 = y_val.div_ceil(vertical_denom) / precinct_y_scale;
    let p5 = try0 / precinct_y_scale;
    let diff_y = p4.checked_sub(p5)?;

    let precincts_wide = resolution_tile.num_precincts_x() as u64;
    if precincts_wide == 0 {
        return None;
    }

    let precinct = diff_x + precincts_wide * diff_y;
    if precinct >= resolution_tile.num_precincts() as u64 {
        return None;
    }

    precinct.try_into().ok()
}
