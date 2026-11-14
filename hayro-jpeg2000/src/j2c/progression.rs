use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use super::codestream::{Header, ProgressionOrder};
use super::tile::{ComponentTile, ResolutionTile, Tile};
use crate::error::{DecodingError, Result};
use core::cmp::Ordering;
use core::iter;

#[derive(Default, Copy, Clone, Debug, PartialEq, Hash, Eq, PartialOrd, Ord)]
pub(crate) struct ProgressionData {
    pub(crate) layer_num: u8,
    pub(crate) resolution: u8,
    pub(crate) component: u8,
    pub(crate) precinct: u64,
}

pub(crate) struct IteratorInput<'a> {
    layers: (u8, u8),
    tile: &'a Tile<'a>,
    resolutions: (u8, u8),
    components: (u8, u8),
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(tile: &'a Tile<'a>) -> Self {
        Self::new_with_custom_bounds(tile, (0, u8::MAX), (0, u8::MAX), (0, u8::MAX))
    }

    pub(crate) fn new_with_custom_bounds(
        tile: &'a Tile<'a>,
        mut resolutions: (u8, u8),
        mut layers: (u8, u8),
        mut components: (u8, u8),
    ) -> Self {
        let max_resolution = tile
            .component_infos
            .iter()
            .map(|c| c.coding_style.parameters.num_resolution_levels)
            .max()
            .unwrap_or(0);
        let max_layer = tile.num_layers;
        let max_component = tile.component_infos.len() as u8;

        resolutions.1 = resolutions.1.min(max_resolution);
        layers.1 = layers.1.min(max_layer);
        components.1 = components.1.min(max_component);

        Self {
            layers,
            tile,
            resolutions,
            components,
        }
    }

    fn min_layer(&self) -> u8 {
        self.layers.0
    }

    fn max_layer(&self) -> u8 {
        self.layers.1
    }

    fn min_resolution(&self) -> u8 {
        self.resolutions.0
    }

    fn total_max_resolution(&self) -> u8 {
        self.resolutions.1
    }

    fn max_resolution(&self, component_idx: u8) -> u8 {
        self.total_max_resolution()
            // `resolutions.1` is the maximum over all components.
            .min(self.tile.component_infos[component_idx as usize].num_resolution_levels())
    }

    fn min_comp(&self) -> u8 {
        self.components.0
    }

    fn max_comp(&self) -> u8 {
        self.components.1
    }

    fn is_empty(&self) -> bool {
        self.layers.0 >= self.layers.1
            || self.resolutions.0 >= self.resolutions.1
            || self.components.0 >= self.components.1
    }

    fn component_tiles(&self) -> Vec<ComponentTile<'a>> {
        self.tile
            .component_infos
            .iter()
            .map(|c| ComponentTile::new(self.tile, c))
            .collect::<Vec<_>>()
    }

    fn num_precincts(
        &self,
        component_tiles: &[ComponentTile<'a>],
        component_idx: u8,
        resolution: u8,
    ) -> u64 {
        if component_idx >= self.max_comp() || resolution >= self.max_resolution(component_idx) {
            return 0;
        }

        ResolutionTile::new(component_tiles[component_idx as usize], resolution).num_precincts()
    }
}

pub(crate) fn progression_iterator<'a>(
    tile: &'a Tile<'a>,
    header: &Header,
) -> Result<Box<dyn Iterator<Item = ProgressionData> + 'a>> {
    let progression_changes = if tile.progression_changes.is_empty() {
        &header.progression_changes
    } else {
        &tile.progression_changes
    };

    if progression_changes.is_empty() {
        return progression_order_iterator(IteratorInput::new(tile), tile.progression_order);
    }

    let clamp = |value: u16| u8::try_from(value).unwrap_or(u8::MAX);

    let iterators = progression_changes
        .iter()
        .map(|change| {
            let input = IteratorInput::new_with_custom_bounds(
                tile,
                (change.resolution_start, change.resolution_end),
                (0, clamp(change.layer_end)),
                (clamp(change.component_start), clamp(change.component_end)),
            );

            progression_order_iterator(input, change.progression_order)
        })
        .collect::<Result<Vec<_>>>()?;

    // Skip packets already emitted by an earlier progression change.
    let mut included = BTreeSet::new();

    Ok(Box::new(
        iterators
            .into_iter()
            .flatten()
            .filter(move |data| included.insert(*data)),
    ))
}

fn progression_order_iterator<'a>(
    input: IteratorInput<'a>,
    progression_order: ProgressionOrder,
) -> Result<Box<dyn Iterator<Item = ProgressionData> + 'a>> {
    Ok(match progression_order {
        ProgressionOrder::LayerResolutionComponentPosition => {
            Box::new(layer_resolution_component_position_progression(input))
        }
        ProgressionOrder::ResolutionLayerComponentPosition => {
            Box::new(resolution_layer_component_position_progression(input))
        }
        ProgressionOrder::ResolutionPositionComponentLayer => Box::new(
            resolution_position_component_layer_progression(input)
                .ok_or(DecodingError::InvalidProgressionIterator)?,
        ),
        ProgressionOrder::PositionComponentResolutionLayer => Box::new(
            position_component_resolution_layer_progression(input)
                .ok_or(DecodingError::InvalidProgressionIterator)?,
        ),
        ProgressionOrder::ComponentPositionResolutionLayer => Box::new(
            component_position_resolution_layer_progression(input)
                .ok_or(DecodingError::InvalidProgressionIterator)?,
        ),
    })
}

pub(crate) fn layer_resolution_component_position_progression<'a>(
    input: IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    let component_tiles = input.component_tiles();

    let mut layer = input.min_layer();
    let mut resolution = input.min_resolution();
    let mut component_idx = input.min_comp();
    let mut num_precincts = input.num_precincts(&component_tiles, component_idx, resolution);
    let mut precinct = 0;

    iter::from_fn(move || {
        if input.is_empty() || layer >= input.max_layer() {
            return None;
        }

        while precinct == num_precincts {
            precinct = 0;
            component_idx += 1;

            if component_idx == input.max_comp() {
                component_idx = input.min_comp();
                resolution += 1;

                if resolution == input.total_max_resolution() {
                    resolution = input.min_resolution();
                    layer += 1;

                    if layer == input.max_layer() {
                        return None;
                    }
                }
            }

            num_precincts = input.num_precincts(&component_tiles, component_idx, resolution);
        }

        let data = ProgressionData {
            layer_num: layer,
            resolution,
            component: component_idx,
            precinct,
        };

        precinct += 1;

        Some(data)
    })
}

pub(crate) fn resolution_layer_component_position_progression<'a>(
    input: IteratorInput<'a>,
) -> impl Iterator<Item = ProgressionData> + 'a {
    let component_tiles = input.component_tiles();

    let mut layer = input.min_layer();
    let mut resolution = input.min_resolution();
    let mut component_idx = input.min_comp();
    let mut num_precincts = input.num_precincts(&component_tiles, component_idx, resolution);
    let mut precinct = 0;

    iter::from_fn(move || {
        if input.is_empty() || resolution >= input.total_max_resolution() {
            return None;
        }

        while precinct == num_precincts {
            precinct = 0;
            component_idx += 1;

            if component_idx == input.max_comp() {
                component_idx = input.min_comp();
                layer += 1;

                if layer == input.max_layer() {
                    layer = input.min_layer();
                    resolution += 1;

                    if resolution == input.total_max_resolution() {
                        return None;
                    }
                }
            }

            num_precincts = input.num_precincts(&component_tiles, component_idx, resolution);
        }

        let data = ProgressionData {
            layer_num: layer,
            resolution,
            component: component_idx,
            precinct,
        };

        precinct += 1;

        Some(data)
    })
}

struct PrecinctStore {
    resolution: u8,
    precinct_y: u32,
    precinct_x: u32,
    component_idx: u8,
    precinct_idx: u64,
}

fn position_progression_common<'a>(
    input: IteratorInput<'a>,
    sort: impl FnMut(&PrecinctStore, &PrecinctStore) -> Ordering,
) -> Option<impl Iterator<Item = ProgressionData> + 'a> {
    let component_tiles = input.component_tiles();
    let mut elements = vec![];

    for component_idx in input.min_comp()..input.max_comp() {
        let component_tile = component_tiles[component_idx as usize];

        for resolution in input.min_resolution()..input.max_resolution(component_idx) {
            let resolution_tile = ResolutionTile::new(component_tile, resolution);

            elements.extend(resolution_tile.precincts()?.map(|d| PrecinctStore {
                precinct_y: d.r_y,
                precinct_x: d.r_x,
                component_idx,
                resolution,
                precinct_idx: d.idx,
            }));
        }
    }

    elements.sort_by(sort);

    Some(elements.into_iter().flat_map(move |e| {
        (input.min_layer()..input.max_layer()).map(move |layer| ProgressionData {
            layer_num: layer,
            resolution: e.resolution,
            component: e.component_idx,
            precinct: e.precinct_idx,
        })
    }))
}

pub(crate) fn resolution_position_component_layer_progression<'a>(
    input: IteratorInput<'a>,
) -> Option<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, |p, s| {
        p.resolution
            .cmp(&s.resolution)
            .then_with(|| p.precinct_y.cmp(&s.precinct_y))
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.component_idx.cmp(&s.component_idx))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}

pub(crate) fn position_component_resolution_layer_progression<'a>(
    input: IteratorInput<'a>,
) -> Option<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, |p, s| {
        p.precinct_y
            .cmp(&s.precinct_y)
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.component_idx.cmp(&s.component_idx))
            .then_with(|| p.resolution.cmp(&s.resolution))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}

pub(crate) fn component_position_resolution_layer_progression<'a>(
    input: IteratorInput<'a>,
) -> Option<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, |p, s| {
        p.component_idx
            .cmp(&s.component_idx)
            .then_with(|| p.precinct_y.cmp(&s.precinct_y))
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.resolution.cmp(&s.resolution))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}
