use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::codestream::{Header, ProgressionOrder};
use super::tile::{ComponentTile, ResolutionTile, Tile};
use crate::cancel::Cancel;
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

#[derive(Clone, Copy)]
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
            .take(usize::from(components.1))
            .skip(usize::from(components.0))
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

    fn cells(self) -> impl Iterator<Item = (usize, usize)> + 'a {
        (self.min_comp()..self.max_comp()).flat_map(move |c| {
            (self.min_resolution()..self.max_resolution(c))
                .map(move |r| (usize::from(c), usize::from(r)))
        })
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

pub(crate) type PacketIterator<'a> = Box<dyn Iterator<Item = Result<ProgressionData>> + 'a>;

pub(crate) fn progression_iterator<'a>(
    tile: &'a Tile<'a>,
    header: &'a Header,
    cancel: Cancel<'a>,
) -> Result<PacketIterator<'a>> {
    let progression_changes = if tile.progression_changes.is_empty() {
        &header.progression_changes
    } else {
        &tile.progression_changes
    };
    let tile_input = IteratorInput::new(tile);
    if progression_changes.is_empty() {
        return Ok(Box::new(
            progression_order_iterator(tile_input, tile.progression_order)?.map(Ok),
        ));
    }
    let clamp = |value: u16| u8::try_from(value).unwrap_or(u8::MAX);
    let inputs = move || {
        progression_changes.iter().map(move |change| {
            let input = IteratorInput::new_with_custom_bounds(
                tile,
                (change.resolution_start, change.resolution_end),
                (0, clamp(change.layer_end)),
                (clamp(change.component_start), clamp(change.component_end)),
            );
            (input, change.progression_order)
        })
    };
    check_precinct_layouts(tile_input, inputs(), cancel)?;
    let mut changes = inputs();
    let mut covered_layer: Vec<_> = tile
        .component_infos
        .iter()
        .map(|c| vec![0_u8; usize::from(c.num_resolution_levels())])
        .collect();
    let mut current: Option<(Packets<'a>, IteratorInput<'a>)> = None;
    let mut next_packet = move || -> Result<Option<ProgressionData>> {
        loop {
            if let Some((packets, input)) = &mut current {
                for packet in packets {
                    if packet.layer_num
                        >= covered_layer[usize::from(packet.component)]
                            [usize::from(packet.resolution)]
                    {
                        return Ok(Some(packet));
                    }
                    cancel.check()?;
                }
                for (c, r) in input.cells() {
                    covered_layer[c][r] = covered_layer[c][r].max(input.max_layer());
                }
                current = None;
            }
            let Some((input, order)) = changes.next() else {
                return Ok(None);
            };
            cancel.check()?;
            if input
                .cells()
                .all(|(c, r)| covered_layer[c][r] >= input.max_layer())
            {
                continue;
            }
            current = Some((progression_order_iterator(input, order)?, input));
        }
    };
    Ok(Box::new(iter::from_fn(move || next_packet().transpose())))
}

fn progression_order_iterator<'a>(
    input: IteratorInput<'a>,
    progression_order: ProgressionOrder,
) -> Result<Packets<'a>> {
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

type Packets<'a> = Box<dyn Iterator<Item = ProgressionData> + 'a>;

fn check_precinct_layouts<'a>(
    input: IteratorInput<'a>,
    changes: impl Iterator<Item = (IteratorInput<'a>, ProgressionOrder)>,
    cancel: Cancel<'_>,
) -> Result<()> {
    let mut checked = vec![0_u64; usize::from(input.max_comp())];
    for (volume, order) in changes {
        cancel.check()?;
        if matches!(
            order,
            ProgressionOrder::LayerResolutionComponentPosition
                | ProgressionOrder::ResolutionLayerComponentPosition
        ) {
            continue;
        }
        for (c, r) in volume.cells() {
            let bit = 1_u64 << r;
            if checked[c] & bit == 0 {
                let component = ComponentTile::new(input.tile, &input.tile.component_infos[c]);
                if ResolutionTile::new(component, r as u8)
                    .precincts()
                    .is_none()
                {
                    return Err(DecodingError::InvalidProgressionIterator.into());
                }
                checked[c] |= bit;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::j2c::codestream::ProgressionChange;
    use crate::j2c::{parse_raw, tile};
    use crate::reader::BitReader;
    use ProgressionOrder::*;
    use alloc::collections::BTreeSet;

    fn reference(tile: &Tile<'_>) -> Vec<ProgressionData> {
        let mut seen = BTreeSet::new();
        tile.progression_changes
            .iter()
            .flat_map(|change| {
                progression_order_iterator(IteratorInput::new(tile), change.progression_order)
                    .unwrap()
                    .filter(move |p| {
                        (usize::from(change.component_start)..usize::from(change.component_end))
                            .contains(&usize::from(p.component))
                            && (change.resolution_start..change.resolution_end)
                                .contains(&p.resolution)
                            && u16::from(p.layer_num) < change.layer_end
                    })
            })
            .filter(|packet| seen.insert(*packet))
            .collect()
    }

    #[test]
    fn covered_positional_change_still_validates_its_layout() {
        let data = include_bytes!("../../tests/streams/packets_poc_main_header_ranges.j2k");
        let stream = parse_raw(data, &crate::DecodeSettings::default()).unwrap();
        let mut tiles = tile::parse(&mut BitReader::new(stream.data), &stream.header).unwrap();
        let tile = &mut tiles[0];
        tile.component_infos[0]
            .coding_style
            .parameters
            .precinct_exponents[1]
            .0 = 0;
        let change = ProgressionChange {
            component_start: 0,
            component_end: 1,
            resolution_start: 1,
            resolution_end: 2,
            layer_end: 1,
            progression_order: LayerResolutionComponentPosition,
        };
        tile.progression_changes = vec![
            change,
            ProgressionChange {
                progression_order: ResolutionPositionComponentLayer,
                ..change
            },
        ];
        assert_eq!(
            progression_iterator(tile, &stream.header, Cancel(None)).err(),
            Some(DecodingError::InvalidProgressionIterator.into())
        );
    }

    #[test]
    fn overlapping_changes_match_first_occurrence() {
        let data = include_bytes!("../../tests/streams/packets_poc_main_header_ranges.j2k");
        let stream = parse_raw(data, &crate::DecodeSettings::default()).unwrap();
        let mut tiles = tile::parse(&mut BitReader::new(stream.data), &stream.header).unwrap();
        let tile = &mut tiles[0];
        tile.num_layers = 3;
        let orders = [
            LayerResolutionComponentPosition,
            ResolutionLayerComponentPosition,
            ResolutionPositionComponentLayer,
            PositionComponentResolutionLayer,
            ComponentPositionResolutionLayer,
        ];
        for first in orders {
            for second in orders {
                let change = ProgressionChange {
                    component_start: 0,
                    component_end: 2,
                    resolution_start: 0,
                    resolution_end: 2,
                    layer_end: 2,
                    progression_order: first,
                };
                tile.progression_changes = vec![
                    change,
                    ProgressionChange {
                        component_start: 1,
                        component_end: 3,
                        resolution_start: 1,
                        resolution_end: 3,
                        layer_end: 1,
                        progression_order: second,
                    },
                    ProgressionChange {
                        layer_end: 3,
                        progression_order: second,
                        ..change
                    },
                ];
                let actual = progression_iterator(tile, &stream.header, Cancel(None))
                    .unwrap()
                    .collect::<Result<Vec<_>>>()
                    .unwrap();
                assert_eq!(actual, reference(tile), "{first:?} then {second:?}");
            }
        }
    }
}
