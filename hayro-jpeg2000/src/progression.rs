use crate::codestream::ComponentInfo;
use crate::tile::{TilePart, TilePartInstance};

#[derive(Default, Copy, Clone, Debug)]
pub(crate) struct ProgressionData {
    layer_num: u16,
    resolution: u16,
    component: u8,
    precinct: u32,
}

pub(crate) struct IteratorInput<'a> {
    layers: u16,
    tile_part: &'a TilePart<'a>,
    component_infos: &'a [ComponentInfo],
    resolutions: u16,
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(
        tile_part: &'a TilePart<'a>,
        component_infos: &'a [ComponentInfo],
        layers: u16,
    ) -> Self {
        let resolutions = component_infos
            .iter()
            .map(|c| c.coding_style_parameters.parameters.num_resolution_levels)
            .max()
            .unwrap();

        Self {
            layers,
            component_infos,
            tile_part,
            resolutions,
        }
    }
}

struct IteratorState<'a> {
    input: IteratorInput<'a>,
    data: ProgressionData,
    first: bool,
    tile_part_instance: TilePartInstance<'a>,
}

impl<'a> IteratorState<'a> {
    fn new(input: IteratorInput<'a>, tile_part_instance: TilePartInstance<'a>) -> Self {
        Self {
            input,
            data: Default::default(),
            first: true,
            tile_part_instance,
        }
    }

    fn advance_layer(&mut self) -> bool {
        self.data.layer_num += 1;

        if self.data.layer_num >= self.input.layers {
            self.data.layer_num = 0;

            true
        } else {
            false
        }
    }

    fn advance_resolution(&mut self) -> bool {
        self.data.resolution += 1;

        let spilled = if self.data.resolution >= self.input.resolutions {
            self.data.resolution = 0;

            true
        } else {
            false
        };

        self.update_tile_part_instance();

        spilled
    }

    fn advance_component(&mut self) -> bool {
        self.data.component += 1;

        let spilled = if self.data.component >= self.input.component_infos.len() as u8 {
            self.data.component = 0;
            true
        } else {
            false
        };

        self.update_tile_part_instance();

        spilled
    }

    fn update_tile_part_instance(&mut self) {
        let component = &self.input.component_infos[self.data.component as usize];
        self.tile_part_instance =
            component.tile_part_instance(self.input.tile_part, self.data.resolution);
    }

    fn advance_precinct(&mut self) -> bool {
        self.data.precinct += 1;

        if self.data.precinct >= self.tile_part_instance.num_precincts() {
            self.data.precinct = 0;

            true
        } else {
            false
        }
    }
}

pub(crate) trait ProgressionIterator<'a>: Iterator<Item = ProgressionData> {
    fn new(iterator_input: IteratorInput<'a>) -> Self;
}

pub(crate) struct ResolutionLevelLayerComponentPositionProgressionIterator<'a> {
    state: IteratorState<'a>,
}

impl Iterator for ResolutionLevelLayerComponentPositionProgressionIterator<'_> {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state.first {
            self.state.first = false;
            return Some(self.state.data);
        }

        if self.state.advance_precinct() {
            if self.state.advance_component() {
                if self.state.advance_layer() {
                    if self.state.advance_resolution() {
                        return None;
                    }
                }
            }
        }

        Some(self.state.data)
    }
}

impl<'a> ProgressionIterator<'a> for ResolutionLevelLayerComponentPositionProgressionIterator<'a> {
    fn new(input: IteratorInput<'a>) -> Self {
        let data = ProgressionData::default();
        let instance = input.component_infos[data.component as usize]
            .tile_part_instance(input.tile_part, data.resolution);

        Self {
            state: IteratorState::new(input, instance),
        }
    }
}
