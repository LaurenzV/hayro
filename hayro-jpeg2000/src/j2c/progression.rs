use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::codestream::{Header, MAX_RESOLUTION_COUNT, ProgressionChange, ProgressionOrder};
use super::tile::{ComponentTile, ResolutionTile, Tile};
use crate::cancel::Cancel;
use crate::error::{DecodingError, Result};
use core::cmp::Ordering;
use core::iter;

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProgressionData {
    pub(crate) layer_num: u8,
    pub(crate) resolution: u8,
    pub(crate) component: usize,
    pub(crate) precinct: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct IteratorInput<'a> {
    layers: (u8, u8),
    tile: &'a Tile<'a>,
    resolutions: (u8, u8),
    components: (usize, usize),
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(tile: &'a Tile<'a>) -> Self {
        let max_resolution = tile
            .component_infos
            .iter()
            .map(|c| c.coding_style.parameters.num_resolution_levels)
            .max()
            .unwrap_or(0);

        Self {
            layers: (0, tile.num_layers),
            tile,
            resolutions: (0, max_resolution),
            components: (0, tile.component_infos.len()),
        }
    }

    fn restrict(&self, change: &ProgressionChange) -> Self {
        let clamp = |value| u8::try_from(value).unwrap_or(u8::MAX);
        Self {
            layers: (0, clamp(change.layer_end).min(self.layers.1)),
            tile: self.tile,
            resolutions: (
                change.resolution_start,
                change.resolution_end.min(self.resolutions.1),
            ),
            components: (
                usize::from(change.component_start).min(self.components.1),
                usize::from(change.component_end).min(self.components.1),
            ),
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

    fn max_resolution(&self, component_idx: usize) -> u8 {
        self.total_max_resolution()
            // `resolutions.1` is the maximum over all components.
            .min(self.tile.component_infos[component_idx].num_resolution_levels())
    }

    fn resolution_bits(&self, component_idx: usize) -> u64 {
        let (min, max) = (self.min_resolution(), self.max_resolution(component_idx));

        if min >= max {
            0
        } else {
            (u64::MAX >> (64 - (max - min))) << min
        }
    }

    fn min_comp(&self) -> usize {
        self.components.0
    }

    fn max_comp(&self) -> usize {
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
        component_idx: usize,
        resolution: u8,
    ) -> u64 {
        if component_idx >= self.max_comp() || resolution >= self.max_resolution(component_idx) {
            return 0;
        }

        ResolutionTile::new(component_tiles[component_idx], resolution).num_precincts()
    }
}

type Packets<'a> = Box<dyn Iterator<Item = ProgressionData> + 'a>;

pub(super) struct ProgressionIterator<'a>(Progression<'a>);

enum Progression<'a> {
    Single(Packets<'a>),
    Changes {
        changes: core::slice::Iter<'a, ProgressionChange>,
        tile: IteratorInput<'a>,
        current: Option<(Packets<'a>, IteratorInput<'a>)>,
        included: Included,
    },
}

impl<'a> ProgressionIterator<'a> {
    pub(super) fn new(tile: &'a Tile<'a>, header: &'a Header, cancel: Cancel<'_>) -> Result<Self> {
        let changes = if tile.progression_changes.is_empty() {
            &header.progression_changes
        } else {
            &tile.progression_changes
        };
        let input = IteratorInput::new(tile);

        Ok(Self(if changes.is_empty() {
            Progression::Single(progression_order_iterator(
                input,
                tile.progression_order,
                cancel,
            )?)
        } else {
            Self::check_precinct_layouts(&input, changes, cancel)?;
            Progression::Changes {
                changes: changes.iter(),
                tile: input,
                current: None,
                included: Included::new(&input),
            }
        }))
    }

    fn check_precinct_layouts(
        tile: &IteratorInput<'_>,
        changes: &[ProgressionChange],
        cancel: Cancel<'_>,
    ) -> Result<()> {
        let mut checked = vec![0_u64; tile.max_comp()];
        for change in changes {
            cancel.check()?;
            if matches!(
                change.progression_order,
                ProgressionOrder::LayerResolutionComponentPosition
                    | ProgressionOrder::ResolutionLayerComponentPosition
            ) {
                continue;
            }
            let input = tile.restrict(change);
            for (component, checked) in checked
                .iter_mut()
                .enumerate()
                .take(input.max_comp())
                .skip(input.min_comp())
            {
                let resolutions = input.resolution_bits(component);
                let mut pending = resolutions & !*checked;
                if pending == 0 {
                    continue;
                }

                cancel.check()?;
                let tile = ComponentTile::new(input.tile, &input.tile.component_infos[component]);
                while pending != 0 {
                    let resolution = pending.trailing_zeros() as u8;
                    if ResolutionTile::new(tile, resolution).precincts().is_none() {
                        return Err(DecodingError::InvalidProgressionIterator.into());
                    }
                    pending &= pending - 1;
                }
                *checked |= resolutions;
            }
        }
        Ok(())
    }

    pub(super) fn next_packet(&mut self, cancel: Cancel<'_>) -> Result<Option<ProgressionData>> {
        let (changes, tile, current, included) = match &mut self.0 {
            Progression::Single(packets) => return Ok(packets.next()),
            Progression::Changes {
                changes,
                tile,
                current,
                included,
            } => (changes, tile, current, included),
        };

        loop {
            if let Some((packets, input)) = current {
                for packet in packets {
                    if !included.contains(&packet) {
                        return Ok(Some(packet));
                    }
                    cancel.check()?;
                }
                included.add(input);
                *current = None;
            }

            let Some(change) = changes.next() else {
                return Ok(None);
            };
            let input = tile.restrict(change);
            if included.covers(&input) {
                cancel.check()?;
                continue;
            }

            *current = Some((
                progression_order_iterator(input, change.progression_order, cancel)?,
                input,
            ));
        }
    }
}

const _: () = assert!(MAX_RESOLUTION_COUNT < 64);

struct Included {
    num_layers: usize,
    // Each (component, layer) word records the resolutions covered by exhausted changes.
    resolutions: Vec<u64>,
}

impl Included {
    fn new(tile: &IteratorInput<'_>) -> Self {
        let num_layers = usize::from(tile.max_layer());

        Self {
            num_layers,
            resolutions: vec![0; tile.max_comp() * num_layers],
        }
    }

    fn index(&self, component_idx: usize, layer: u8) -> usize {
        component_idx * self.num_layers + usize::from(layer)
    }

    fn contains(&self, data: &ProgressionData) -> bool {
        self.resolutions[self.index(data.component, data.layer_num)] >> data.resolution & 1 == 1
    }

    fn covers(&self, input: &IteratorInput<'_>) -> bool {
        // Every change starts at layer zero, so coverage at the top implies coverage below.
        let Some(top_layer) = input.max_layer().checked_sub(1) else {
            return true;
        };

        (input.min_comp()..input.max_comp()).all(|component_idx| {
            let resolutions = input.resolution_bits(component_idx);

            self.resolutions[self.index(component_idx, top_layer)] & resolutions == resolutions
        })
    }

    fn add(&mut self, input: &IteratorInput<'_>) {
        for component_idx in input.min_comp()..input.max_comp() {
            let resolutions = input.resolution_bits(component_idx);

            for layer in input.min_layer()..input.max_layer() {
                let index = self.index(component_idx, layer);
                self.resolutions[index] |= resolutions;
            }
        }
    }
}

fn progression_order_iterator<'a>(
    input: IteratorInput<'a>,
    progression_order: ProgressionOrder,
    cancel: Cancel<'_>,
) -> Result<Packets<'a>> {
    Ok(match progression_order {
        ProgressionOrder::LayerResolutionComponentPosition => {
            Box::new(layer_resolution_component_position_progression(input))
        }
        ProgressionOrder::ResolutionLayerComponentPosition => {
            Box::new(resolution_layer_component_position_progression(input))
        }
        ProgressionOrder::ResolutionPositionComponentLayer => Box::new(
            resolution_position_component_layer_progression(input, cancel)?,
        ),
        ProgressionOrder::PositionComponentResolutionLayer => Box::new(
            position_component_resolution_layer_progression(input, cancel)?,
        ),
        ProgressionOrder::ComponentPositionResolutionLayer => Box::new(
            component_position_resolution_layer_progression(input, cancel)?,
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
    component_idx: u16,
    precinct_idx: u64,
}

fn position_progression_common<'a>(
    input: IteratorInput<'a>,
    cancel: Cancel<'_>,
    sort: impl FnMut(&PrecinctStore, &PrecinctStore) -> Ordering,
) -> Result<impl Iterator<Item = ProgressionData> + 'a> {
    let mut elements = vec![];

    for component_idx in input.min_comp()..input.max_comp() {
        let component_tile =
            ComponentTile::new(input.tile, &input.tile.component_infos[component_idx]);
        let resolutions = input.min_resolution()..input.max_resolution(component_idx);
        let component_idx = component_idx as u16;

        for resolution in resolutions {
            let resolution_tile = ResolutionTile::new(component_tile, resolution);

            cancel.check()?;
            let precincts = resolution_tile
                .precincts()
                .ok_or(DecodingError::InvalidProgressionIterator)?;
            for precinct in precincts {
                if elements.len().is_multiple_of(crate::cancel::CHUNK) {
                    cancel.check()?;
                }
                elements.push(PrecinctStore {
                    precinct_y: precinct.r_y,
                    precinct_x: precinct.r_x,
                    component_idx,
                    resolution,
                    precinct_idx: precinct.idx,
                });
            }
        }
    }

    elements.sort_by(sort);
    cancel.check()?;

    Ok(elements.into_iter().flat_map(move |e| {
        (input.min_layer()..input.max_layer()).map(move |layer| ProgressionData {
            layer_num: layer,
            resolution: e.resolution,
            component: usize::from(e.component_idx),
            precinct: e.precinct_idx,
        })
    }))
}

pub(crate) fn resolution_position_component_layer_progression<'a>(
    input: IteratorInput<'a>,
    cancel: Cancel<'_>,
) -> Result<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, cancel, |p, s| {
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
    cancel: Cancel<'_>,
) -> Result<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, cancel, |p, s| {
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
    cancel: Cancel<'_>,
) -> Result<impl Iterator<Item = ProgressionData> + 'a> {
    position_progression_common(input, cancel, |p, s| {
        p.component_idx
            .cmp(&s.component_idx)
            .then_with(|| p.precinct_y.cmp(&s.precinct_y))
            .then_with(|| p.precinct_x.cmp(&s.precinct_x))
            .then_with(|| p.resolution.cmp(&s.resolution))
            .then_with(|| p.precinct_idx.cmp(&s.precinct_idx))
    })
}

#[cfg(test)]
mod tests {
    use crate::{DecodeSettings, DecoderContext, Image};
    use alloc::vec;
    use alloc::vec::Vec;

    fn marker(code: u16, body: &[u8]) -> Vec<u8> {
        let mut marker = code.to_be_bytes().to_vec();
        marker.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        marker.extend(body);
        marker
    }

    fn position_order_stream(components: u8, levels: u8, side: u32, tile_header: &[u8]) -> Vec<u8> {
        let mut siz = vec![0, 0];
        for value in [side, side, 0, 0, side, side, 0, 0] {
            siz.extend(value.to_be_bytes());
        }
        siz.extend(u16::from(components).to_be_bytes());
        siz.extend([7, 1, 1].repeat(usize::from(components)));
        let mut cod = vec![1, 2, 0, 1, 0, levels, 4, 4, 0, 1];
        cod.extend(vec![0x11; usize::from(levels) + 1]);
        let mut qcd = vec![0x40];
        qcd.extend(vec![0x48; 3 * usize::from(levels) + 1]);
        let change = [0, 0, 0, 1, levels + 1, components, 2];
        let packets = usize::from(components) * (usize::from(levels) + 1);
        let data = vec![0; packets + 1];
        let mut sot = vec![0, 0];
        sot.extend(
            u32::try_from(12 + tile_header.len() + 2 + data.len())
                .unwrap()
                .to_be_bytes(),
        );
        sot.extend([0, 1]);

        let mut stream = vec![0xFF, 0x4F];
        stream.extend(marker(0xFF51, &siz));
        stream.extend(marker(0xFF52, &cod));
        stream.extend(marker(0xFF5C, &qcd));
        stream.extend(marker(0xFF5F, &change));
        stream.extend(marker(0xFF90, &sot));
        stream.extend(tile_header);
        stream.extend([0xFF, 0x93]);
        stream.extend(data);
        stream.extend([0xFF, 0xD9]);
        stream
    }

    #[test]
    fn invalid_precinct_in_a_position_order_change() {
        use super::ProgressionOrder::*;
        use crate::{DecodeError, DecodingError};

        let data = position_order_stream(1, 1, 1, &[]);
        let mut image = Image::new(&data, &DecodeSettings::default()).unwrap();
        image.header.component_infos[0]
            .coding_style
            .parameters
            .precinct_exponents[1]
            .0 = 0;
        let change = &mut image.header.progression_changes[0];
        change.progression_order = LayerResolutionComponentPosition;
        let mut positional = *change;
        positional.progression_order = ResolutionPositionComponentLayer;

        for (end, expected) in [
            (1, DecodingError::InvalidPrecinct),
            (2, DecodingError::InvalidProgressionIterator),
        ] {
            positional.resolution_end = end;
            image.header.progression_changes.truncate(1);
            image.header.progression_changes.push(positional);
            assert_eq!(
                image.decode(&mut DecoderContext::default()).err(),
                Some(DecodeError::Decoding(expected))
            );
        }
    }

    #[test]
    fn overflowing_stride_in_an_empty_resolution() {
        use crate::{DecodeError, DecodingError};

        let data = position_order_stream(1, 17, 2, &[]);
        let mut image = Image::new(&data, &DecodeSettings::default()).unwrap();
        image.header.size_data.image_area_x_offset = 1;
        image.header.component_infos[0]
            .coding_style
            .parameters
            .precinct_exponents[0]
            .0 = 15;
        for (start, expected) in [
            (0, DecodingError::InvalidProgressionIterator),
            (1, DecodingError::InvalidPrecinct),
        ] {
            image.header.progression_changes[0].resolution_start = start;
            assert_eq!(
                image.decode(&mut DecoderContext::default()).err(),
                Some(DecodeError::Decoding(expected))
            );
        }
    }

    #[test]
    fn positional_layout_precedes_reduced_tile_resolution_error() {
        use crate::{DecodeError, DecodingError};

        let tile_cod = marker(0xff52, &[1, 2, 0, 1, 0, 1, 4, 4, 0, 1, 0x11, 0x10]);
        let data = position_order_stream(1, 2, 4, &tile_cod);
        for strict in [false, true] {
            let settings = DecodeSettings {
                strict,
                target_resolution: Some((1, 1)),
                ..Default::default()
            };
            let image = Image::new(&data, &settings).unwrap();
            assert_eq!(image.header.skipped_resolution_levels, 2);
            assert_eq!(
                image.decode(&mut DecoderContext::default()).err(),
                Some(DecodeError::Decoding(
                    DecodingError::InvalidProgressionIterator
                ))
            );
        }
    }

    #[test]
    fn positional_component_indices_do_not_wrap() {
        use crate::{DecodeError, DecodingError};

        let data = position_order_stream(1, 1, 1, &[]);
        let mut image = Image::new(&data, &DecodeSettings::default()).unwrap();
        let infos = &mut image.header.component_infos;
        infos.resize(257, infos[0].clone());
        infos[256].coding_style.parameters.precinct_exponents[1].0 = 0;
        assert_eq!(
            image.decode(&mut DecoderContext::default()).err(),
            Some(DecodeError::Decoding(DecodingError::InvalidPrecinct))
        );
        image.header.progression_changes[0].component_start = 256;
        image.header.progression_changes[0].component_end = 257;
        assert_eq!(
            image.decode(&mut DecoderContext::default()).err(),
            Some(DecodeError::Decoding(
                DecodingError::InvalidProgressionIterator
            ))
        );
        image.header.component_infos[256]
            .coding_style
            .parameters
            .precinct_exponents[1]
            .0 = 1;
        let tiles = crate::j2c::tile::parse(
            &mut crate::reader::BitReader::new(image.codestream),
            &image.header,
        )
        .unwrap();
        let cancel = super::Cancel(None);
        let mut packets =
            super::ProgressionIterator::new(&tiles[0], &image.header, cancel).unwrap();
        assert_eq!(packets.next_packet(cancel).unwrap().unwrap().component, 256);
    }

    #[test]
    fn cancellation_while_building_a_later_position_order() {
        use super::*;
        use crate::j2c::{parse_raw, tile};
        use crate::reader::BitReader;
        use crate::{CancelHandle, DecodeError};

        let data = position_order_stream(2, 1, 16, &[]);
        let stream = parse_raw(&data, &DecodeSettings::default()).unwrap();
        let mut tiles = tile::parse(&mut BitReader::new(stream.data), &stream.header).unwrap();
        let tile = &mut tiles[0];
        let all = stream.header.progression_changes[0];
        tile.progression_changes = vec![
            ProgressionChange {
                component_end: 1,
                resolution_end: 1,
                progression_order: ProgressionOrder::LayerResolutionComponentPosition,
                ..all
            },
            ProgressionChange {
                component_start: 1,
                ..all
            },
        ];
        let mut packets = ProgressionIterator::new(tile, &stream.header, Cancel(None)).unwrap();
        let first = ResolutionTile::new(ComponentTile::new(tile, &tile.component_infos[0]), 0);
        for _ in 0..first.num_precincts() {
            assert_eq!(
                packets
                    .next_packet(Cancel(None))
                    .unwrap()
                    .unwrap()
                    .component,
                0
            );
        }
        let handle = CancelHandle::new(|| true);
        assert_eq!(
            packets.next_packet(Cancel(Some(&handle))),
            Err(DecodeError::Cancelled)
        );
    }

    #[test]
    fn cancellation_while_skipping_changes_and_packets() {
        use super::*;
        use crate::j2c::{parse_raw, tile};
        use crate::reader::BitReader;
        use crate::{CancelHandle, DecodeError};
        use core::sync::atomic::{AtomicUsize, Ordering};

        let data = position_order_stream(2, 1, 1, &[]);
        let stream = parse_raw(&data, &DecodeSettings::default()).unwrap();
        let mut tiles = tile::parse(&mut BitReader::new(stream.data), &stream.header).unwrap();
        let tile = &mut tiles[0];
        tile.num_layers = 2;
        let first = ProgressionChange {
            component_end: 1,
            progression_order: ProgressionOrder::LayerResolutionComponentPosition,
            ..stream.header.progression_changes[0]
        };

        for changes in [
            vec![first; 3],
            vec![
                first,
                ProgressionChange {
                    layer_end: 2,
                    ..first
                },
            ],
        ] {
            tile.progression_changes = changes;
            let mut packets = ProgressionIterator::new(tile, &stream.header, Cancel(None)).unwrap();
            for _ in 0..2 {
                assert!(packets.next_packet(Cancel(None)).unwrap().is_some());
            }
            let polls = AtomicUsize::new(0);
            let handle = CancelHandle::new(move || polls.fetch_add(1, Ordering::Relaxed) == 1);
            assert!(matches!(
                packets.next_packet(Cancel(Some(&handle))),
                Err(DecodeError::Cancelled)
            ));
        }
    }
}
