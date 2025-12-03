//! Progression iterators, defined in Section B.12.
//!
//! A progression iterator essentially yields tuples of
//! (layer_num, resolution, component, precinct) in a specific order that
//! determines in which order the data appears in the codestream.

use super::tile::{ComponentTile, ResolutionTile, Tile};
use std::cmp::Ordering;
use std::iter;

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
pub(crate) fn layer_resolution_component_position_progression<'a>(
    input: &'a IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    let num_components = input.tile.component_infos.len();

    let component_tiles = input.component_tiles();

    let mut layer = 0;
    let mut resolution = 0;
    let mut component_idx = 0;
    let mut resolution_tile = ResolutionTile::new(component_tiles[0], resolution);
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
pub(crate) fn resolution_layer_component_position_progression<'a>(
    input: &'a IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
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

// The formula for the remaining three progressions looks very intimidating.
// But really, all they boil down to is that we need to determine all precinct
// indices for each component/resolution combination and sort them by ascending
// y/x coordinate on the reference grid. Other than that, they can be treated
// exactly the same, except that the sort order precedence of the fields change.

// Note that the order of fields here is important!
struct PrecinctStore {
    resolution: u16,
    precinct_y: u32,
    precinct_x: u32,
    component_idx: u8,
    precinct_idx: u32,
}

fn position_progression_common<'a>(
    input: &'a IteratorInput<'a>,
    sort: impl FnMut(&PrecinctStore, &PrecinctStore) -> Ordering,
) -> impl Iterator<Item = ProgressionData> + 'a {
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

    elements.sort_by(sort);

    elements.into_iter().flat_map(|e| {
        (0..input.layers).map(move |layer| ProgressionData {
            layer_num: layer,
            resolution: e.resolution,
            component: e.component_idx,
            precinct: e.precinct_idx,
        })
    })
}

/// B.12.1.3 Resolution level-position-component-layer progression.
pub(crate) fn resolution_position_component_layer_progression<'a>(
    input: &'a IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    position_progression_common(input, |p, s| {
        p.resolution
            .cmp(&s.resolution)
            .then_with(|| p.precinct_y.cmp(&s.precinct_y))
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.component_idx.cmp(&s.component_idx))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}

/// B.12.1.4 Position-component-resolution level-layer progression.
pub(crate) fn position_component_resolution_layer_progression<'a>(
    input: &'a IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    position_progression_common(input, |p, s| {
        p.precinct_y
            .cmp(&s.precinct_y)
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.component_idx.cmp(&s.component_idx))
            .then_with(|| p.resolution.cmp(&s.resolution))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}

/// B.12.1.5 Component-position-resolution level-layer progression.
pub(crate) fn component_position_resolution_layer_progression<'a>(
    input: &'a IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    position_progression_common(input, |p, s| {
        p.component_idx
            .cmp(&s.component_idx)
            .then_with(|| p.precinct_y.cmp(&s.precinct_y))
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.resolution.cmp(&s.resolution))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}
