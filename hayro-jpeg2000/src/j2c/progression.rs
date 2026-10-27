//! Progression iterators, defined in Section B.12.
//!
//! A progression iterator essentially yields tuples of
//! (`layer_num`, resolution, component, precinct) in a specific order that
//! determines in which order the data appears in the codestream.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::codestream::{Header, MAX_RESOLUTION_COUNT, ProgressionOrder};
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

#[derive(Clone, Copy)]
pub(crate) struct IteratorInput<'a> {
    layers: (u8, u8),
    tile: &'a Tile<'a>,
    resolutions: (u8, u8),
    components: (u8, u8),
}

impl<'a> IteratorInput<'a> {
    /// All packets of the tile.
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
            components: (0, tile.component_infos.len() as u8),
        }
    }

    /// The packets of this input within the given bounds.
    fn restrict(&self, resolutions: (u8, u8), layers: (u8, u8), components: (u8, u8)) -> Self {
        Self {
            layers: (layers.0, layers.1.min(self.layers.1)),
            tile: self.tile,
            resolutions: (resolutions.0, resolutions.1.min(self.resolutions.1)),
            components: (components.0, components.1.min(self.components.1)),
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
            // It's possible that the different component tiles have different resolution levels
            // (self.resolutions.1 stores the maximum across all component tiles), so
            // take the minimum of both.
            .min(self.tile.component_infos[component_idx as usize].num_resolution_levels())
    }

    /// The resolutions of the given component as a bit set.
    fn resolution_bits(&self, component_idx: u8) -> u64 {
        let (min, max) = (self.min_resolution(), self.max_resolution(component_idx));

        if min >= max {
            0
        } else {
            (u64::MAX >> (64 - (max - min))) << min
        }
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

    /// The number of precincts of the given component at the given
    /// resolution, or 0 if the component doesn't have that resolution.
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

/// Create the iterator that yields the packets of the tile in codestream order,
/// taking progression order changes into account (B.12.3).
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
    let tile_input = IteratorInput::new(tile);

    // A change's iterator is built once the previous one is used up: one in a
    // position order holds every precinct it covers, and changes may overlap.
    // Precincts that can't be laid out end the packets, but they already fail
    // the tile when its precincts are built.
    let mut changes = progression_changes.clone().into_iter();
    let mut current: Option<(
        Box<dyn Iterator<Item = ProgressionData> + 'a>,
        IteratorInput<'a>,
    )> = None;
    let mut included = Included::new(&tile_input);

    Ok(Box::new(iter::from_fn(move || {
        loop {
            if let Some((iterator, input)) = &mut current {
                if let Some(data) = iterator.find(|data| !included.contains(data)) {
                    return Some(data);
                }

                included.add(input);
                current = None;
            }

            let change = changes.next()?;
            let input = tile_input.restrict(
                (change.resolution_start, change.resolution_end),
                (0, clamp(change.layer_end)),
                (clamp(change.component_start), clamp(change.component_end)),
            );

            if included.covers(&input) {
                continue;
            }

            current = Some((
                progression_order_iterator(input, change.progression_order).ok()?,
                input,
            ));
        }
    })))
}

// Resolutions are bits of a `u64`.
const _: () = assert!(MAX_RESOLUTION_COUNT < 64);

/// The packets that the used-up progression order changes included, which
/// must not be included again (B.12.3).
///
/// Each change starts at layer 0 and covers every precinct of its resolutions
/// and components, so for each component and layer the included packets are
/// those of a set of resolutions. Where a layer is included, so are the layers
/// below it.
struct Included {
    num_layers: usize,
    /// Bit `r` of entry `c * num_layers + l` is set once layer `l` of
    /// resolution `r` of component `c` is included.
    resolutions: Vec<u64>,
}

impl Included {
    fn new(tile: &IteratorInput<'_>) -> Self {
        let num_layers = usize::from(tile.max_layer());

        Self {
            num_layers,
            resolutions: vec![0; usize::from(tile.max_comp()) * num_layers],
        }
    }

    fn index(&self, component_idx: u8, layer: u8) -> usize {
        usize::from(component_idx) * self.num_layers + usize::from(layer)
    }

    fn contains(&self, data: &ProgressionData) -> bool {
        self.resolutions[self.index(data.component, data.layer_num)] >> data.resolution & 1 == 1
    }

    /// Whether every packet of the input is included, checked at its top
    /// layer: one test per component.
    fn covers(&self, input: &IteratorInput<'_>) -> bool {
        let Some(top_layer) = input.max_layer().checked_sub(1) else {
            return true;
        };

        (input.min_comp()..input.max_comp()).all(|component_idx| {
            #[cfg(test)]
            tests::COVER_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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

/// B.12.1.1 Layer-resolution level-component-position progression.
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

/// B.12.1.2 Resolution level-layer-component-position progression.
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

// The formula for the remaining three progressions looks very intimidating.
// But really, all they boil down to is that we need to determine all precinct
// indices for each component/resolution combination and sort them by ascending
// y/x coordinate on the reference grid. Other than that, they can be treated
// exactly the same, except that the sort order precedence of the fields change.

// Note that the order of fields here is important!
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

/// B.12.1.3 Resolution level-position-component-layer progression.
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

/// B.12.1.4 Position-component-resolution level-layer progression.
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

/// B.12.1.5 Component-position-resolution level-layer progression.
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

#[cfg(test)]
mod tests {
    use crate::{DecodeSettings, DecoderContext, Image};
    use alloc::vec;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// How many entries of the included packets the checks for skipping a
    /// progression order change read.
    pub(super) static COVER_READS: AtomicUsize = AtomicUsize::new(0);

    fn marker(code: u16, body: &[u8]) -> Vec<u8> {
        let mut marker = code.to_be_bytes().to_vec();
        marker.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        marker.extend(body);
        marker
    }

    /// A 1 × 1 image with `components` components, `levels` decomposition
    /// levels (one precinct per resolution), one layer and `changes`
    /// identical RPCL changes over all of it. Every packet is empty, and one
    /// more byte follows them, so the decoder reaches every change.
    fn repeated_changes(components: u8, levels: u8, changes: usize) -> Vec<u8> {
        let mut siz = vec![0, 0];
        for value in [1_u32, 1, 0, 0, 1, 1, 0, 0] {
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
        sot.extend(u32::try_from(12 + 2 + data.len()).unwrap().to_be_bytes());
        sot.extend([0, 1]);

        let mut stream = vec![0xFF, 0x4F];
        stream.extend(marker(0xFF51, &siz));
        stream.extend(marker(0xFF52, &cod));
        stream.extend(marker(0xFF5C, &qcd));
        stream.extend(marker(0xFF5F, &change.repeat(changes)));
        stream.extend(marker(0xFF90, &sot));
        stream.extend([0xFF, 0x93]);
        stream.extend(data);
        stream.extend([0xFF, 0xD9]);
        stream
    }

    #[test]
    fn skipping_a_change_reads_each_component_once() {
        let (components, changes) = (255, 1000);
        let data = repeated_changes(components, 30, changes);
        let image = Image::new(&data, &DecodeSettings::default()).unwrap();
        let reads = COVER_READS.load(Ordering::Relaxed);
        image.decode(&mut DecoderContext::default()).unwrap();
        let reads = COVER_READS.load(Ordering::Relaxed) - reads;

        assert!(
            reads <= changes * usize::from(components),
            "skipping {changes} changes over {components} components and 31 resolutions read {reads} entries"
        );
    }
}
