use crate::codestream::ComponentInfo;
use crate::tile::{Tile, TileInstance};

#[derive(Default, Copy, Clone, Debug)]
pub(crate) struct ProgressionData {
    pub(crate) layer_num: u16,
    pub(crate) resolution: u16,
    pub(crate) component: u8,
    pub(crate) precinct: u32,
}

pub(crate) struct IteratorInput<'a> {
    layers: u16,
    tile: &'a Tile<'a>,
    component_infos: &'a [ComponentInfo],
    resolutions: u16,
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(
        tile: &'a Tile<'a>,
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
            tile,
            resolutions,
        }
    }
}

struct IteratorState<'a> {
    input: IteratorInput<'a>,
    data: ProgressionData,
    first: bool,
    tile_part_instance: TileInstance<'a>,
}

impl<'a> IteratorState<'a> {
    fn new(input: IteratorInput<'a>, tile_part_instance: TileInstance<'a>) -> Self {
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
        self.tile_part_instance = component.tile_instance(self.input.tile, self.data.resolution);
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

pub(crate) struct LayerResolutionLevelComponentPositionProgressionIterator<'a> {
    state: IteratorState<'a>,
}

impl Iterator for LayerResolutionLevelComponentPositionProgressionIterator<'_> {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state.first {
            self.state.first = false;
            return Some(self.state.data);
        }

        if self.state.advance_precinct()
            && self.state.advance_component()
            && self.state.advance_resolution()
            && self.state.advance_layer()
        {
            return None;
        }

        Some(self.state.data)
    }
}

impl<'a> ProgressionIterator<'a> for LayerResolutionLevelComponentPositionProgressionIterator<'a> {
    fn new(input: IteratorInput<'a>) -> Self {
        let data = ProgressionData::default();
        let instance = input.component_infos[data.component as usize]
            .tile_instance(input.tile, data.resolution);

        Self {
            state: IteratorState::new(input, instance),
        }
    }
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

        if self.state.advance_precinct()
            && self.state.advance_component()
            && self.state.advance_layer()
            && self.state.advance_resolution()
        {
            return None;
        }

        Some(self.state.data)
    }
}

impl<'a> ProgressionIterator<'a> for ResolutionLevelLayerComponentPositionProgressionIterator<'a> {
    fn new(input: IteratorInput<'a>) -> Self {
        let data = ProgressionData::default();
        let instance = input.component_infos[data.component as usize]
            .tile_instance(input.tile, data.resolution);

        Self {
            state: IteratorState::new(input, instance),
        }
    }
}

pub(crate) struct ResolutionPositionComponentLayerProgressionIterator {
    sequence: Vec<ProgressionData>,
    index: usize,
}

impl Iterator for ResolutionPositionComponentLayerProgressionIterator {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.sequence.len() {
            None
        } else {
            let value = self.sequence[self.index];
            self.index += 1;
            Some(value)
        }
    }
}

impl<'a> ProgressionIterator<'a> for ResolutionPositionComponentLayerProgressionIterator {
    fn new(input: IteratorInput<'a>) -> Self {
        let sequence = build_resolution_position_component_layer_sequence(&input);

        Self { sequence, index: 0 }
    }
}

pub(crate) struct PositionComponentResolutionLayerProgressionIterator {
    sequence: Vec<ProgressionData>,
    index: usize,
}

impl Iterator for PositionComponentResolutionLayerProgressionIterator {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.sequence.len() {
            None
        } else {
            let value = self.sequence[self.index];
            self.index += 1;
            Some(value)
        }
    }
}

impl<'a> ProgressionIterator<'a> for PositionComponentResolutionLayerProgressionIterator {
    fn new(input: IteratorInput<'a>) -> Self {
        let sequence = build_position_component_resolution_layer_sequence(&input);

        Self { sequence, index: 0 }
    }
}

pub(crate) struct ComponentPositionResolutionLayerProgressionIterator {
    sequence: Vec<ProgressionData>,
    index: usize,
}

impl Iterator for ComponentPositionResolutionLayerProgressionIterator {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.sequence.len() {
            None
        } else {
            let value = self.sequence[self.index];
            self.index += 1;
            Some(value)
        }
    }
}

impl<'a> ProgressionIterator<'a> for ComponentPositionResolutionLayerProgressionIterator {
    fn new(input: IteratorInput<'a>) -> Self {
        let sequence = build_component_position_resolution_layer_sequence(&input);

        Self { sequence, index: 0 }
    }
}

struct ComponentResolutionData<'a> {
    tile_instance: TileInstance<'a>,
    precinct_count: u32,
    precincts_wide: u32,
}

fn build_resolution_position_component_layer_sequence<'a>(
    input: &IteratorInput<'a>,
) -> Vec<ProgressionData> {
    let tile_rect = input.tile.rect;
    let tx0 = tile_rect.x0 as u64;
    let tx1 = tile_rect.x1 as u64;
    let ty0 = tile_rect.y0 as u64;
    let ty1 = tile_rect.y1 as u64;

    let component_data = prepare_component_resolution_data(input);
    let mut sequence = Vec::new();

    for r in 0..input.resolutions {
        let r_idx = r as usize;

        for y in ty0..ty1 {
            for x in tx0..tx1 {
                for component_idx in 0..input.component_infos.len() {
                    let Some(res_data) = component_data[component_idx][r_idx].as_ref() else {
                        continue;
                    };

                    if res_data.precinct_count == 0 {
                        continue;
                    }

                    if !position_matches(
                        x,
                        y,
                        input,
                        component_idx,
                        &res_data.tile_instance,
                        r,
                        tx0,
                        ty0,
                    ) {
                        continue;
                    }

                    if let Some(k) = compute_precinct_index(x, y, input, component_idx, res_data, r)
                    {
                        if k < res_data.precinct_count {
                            for layer in 0..input.layers {
                                sequence.push(ProgressionData {
                                    layer_num: layer,
                                    resolution: r,
                                    component: component_idx as u8,
                                    precinct: k,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    sequence
}

fn build_position_component_resolution_layer_sequence<'a>(
    input: &IteratorInput<'a>,
) -> Vec<ProgressionData> {
    let tile_rect = input.tile.rect;
    let tx0 = tile_rect.x0 as u64;
    let tx1 = tile_rect.x1 as u64;
    let ty0 = tile_rect.y0 as u64;
    let ty1 = tile_rect.y1 as u64;

    let component_data = prepare_component_resolution_data(input);
    let mut sequence = Vec::new();

    for y in ty0..ty1 {
        for x in tx0..tx1 {
            for (component_idx, component_info) in input.component_infos.iter().enumerate() {
                let num_resolution_levels = component_info
                    .coding_style_parameters
                    .parameters
                    .num_resolution_levels;

                for r in 0..num_resolution_levels {
                    let Some(res_data) = component_data[component_idx][r as usize].as_ref() else {
                        continue;
                    };

                    if res_data.precinct_count == 0 {
                        continue;
                    }

                    if !position_matches(
                        x,
                        y,
                        input,
                        component_idx,
                        &res_data.tile_instance,
                        r,
                        tx0,
                        ty0,
                    ) {
                        continue;
                    }

                    if let Some(k) = compute_precinct_index(x, y, input, component_idx, res_data, r)
                    {
                        if k < res_data.precinct_count {
                            for layer in 0..input.layers {
                                sequence.push(ProgressionData {
                                    layer_num: layer,
                                    resolution: r,
                                    component: component_idx as u8,
                                    precinct: k,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    sequence
}

fn build_component_position_resolution_layer_sequence<'a>(
    input: &IteratorInput<'a>,
) -> Vec<ProgressionData> {
    let tile_rect = input.tile.rect;
    let tx0 = tile_rect.x0 as u64;
    let tx1 = tile_rect.x1 as u64;
    let ty0 = tile_rect.y0 as u64;
    let ty1 = tile_rect.y1 as u64;

    let component_data = prepare_component_resolution_data(input);
    let mut sequence = Vec::new();

    for (component_idx, component_info) in input.component_infos.iter().enumerate() {
        let num_resolution_levels = component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels;

        for y in ty0..ty1 {
            for x in tx0..tx1 {
                for r in 0..num_resolution_levels {
                    let Some(res_data) = component_data[component_idx][r as usize].as_ref() else {
                        continue;
                    };

                    if res_data.precinct_count == 0 {
                        continue;
                    }

                    if !position_matches(
                        x,
                        y,
                        input,
                        component_idx,
                        &res_data.tile_instance,
                        r,
                        tx0,
                        ty0,
                    ) {
                        continue;
                    }

                    if let Some(k) = compute_precinct_index(x, y, input, component_idx, res_data, r)
                    {
                        if k < res_data.precinct_count {
                            for layer in 0..input.layers {
                                sequence.push(ProgressionData {
                                    layer_num: layer,
                                    resolution: r,
                                    component: component_idx as u8,
                                    precinct: k,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    sequence
}

fn prepare_component_resolution_data<'a>(
    input: &IteratorInput<'a>,
) -> Vec<Vec<Option<ComponentResolutionData<'a>>>> {
    let component_count = input.component_infos.len();
    let max_resolutions = input.resolutions as usize;
    let mut data = (0..component_count)
        .map(|_| {
            std::iter::repeat_with(|| None)
                .take(max_resolutions)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for (component_idx, component_info) in input.component_infos.iter().enumerate() {
        let num_resolution_levels = component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels;

        for r in 0..num_resolution_levels {
            let tile_instance = component_info.tile_instance(input.tile, r);
            let precincts_wide = tile_instance.num_precincts_x();
            let precincts_high = tile_instance.num_precincts_y();
            let precinct_count = precincts_wide.saturating_mul(precincts_high);

            data[component_idx][r as usize] = Some(ComponentResolutionData {
                tile_instance,
                precinct_count,
                precincts_wide,
            });
        }
    }

    data
}

fn position_matches(
    x: u64,
    y: u64,
    input: &IteratorInput<'_>,
    component_idx: usize,
    tile_instance: &TileInstance,
    resolution: u16,
    tx0: u64,
    ty0: u64,
) -> bool {
    let component = &input.component_infos[component_idx];
    let params = &component.coding_style_parameters.parameters;

    if resolution as usize >= params.precinct_exponents.len() {
        return false;
    }

    let (ppx_u8, ppy_u8) = params.precinct_exponents[resolution as usize];
    let n_l = params.num_decomposition_levels as u32;
    if resolution as u32 > n_l {
        return false;
    }

    let Some(scale_factor) = pow2_u64(n_l - resolution as u32) else {
        return false;
    };

    let shift_x = ppx_u8 as u32 + n_l - resolution as u32;
    let shift_y = ppy_u8 as u32 + n_l - resolution as u32;
    let Some(x_period_factor) = pow2_u64(shift_x) else {
        return false;
    };
    let Some(y_period_factor) = pow2_u64(shift_y) else {
        return false;
    };

    let x_rsiz = component.size_info.horizontal_resolution as u64;
    let y_rsiz = component.size_info.vertical_resolution as u64;
    if x_rsiz == 0 || y_rsiz == 0 {
        return false;
    }

    let x_period = x_period_factor.saturating_mul(x_rsiz);
    let y_period = y_period_factor.saturating_mul(y_rsiz);
    if x_period == 0 || y_period == 0 {
        return false;
    }

    let trx0 = tile_instance.resolution_transformed_rect.x0 as u64;
    let try0 = tile_instance.resolution_transformed_rect.y0 as u64;

    let y_condition =
        (y % y_period == 0) || ((y == ty0) && ((try0 * scale_factor) % y_period != 0));

    if !y_condition {
        return false;
    }

    let x_condition =
        (x % x_period == 0) || ((x == tx0) && ((trx0 * scale_factor) % x_period != 0));

    x_condition
}

fn compute_precinct_index(
    x: u64,
    y: u64,
    input: &IteratorInput<'_>,
    component_idx: usize,
    res_data: &ComponentResolutionData<'_>,
    resolution: u16,
) -> Option<u32> {
    let component = &input.component_infos[component_idx];
    let params = &component.coding_style_parameters.parameters;

    if resolution as usize >= params.precinct_exponents.len() {
        return None;
    }

    let (ppx_u8, ppy_u8) = params.precinct_exponents[resolution as usize];
    let n_l = params.num_decomposition_levels as u32;
    if resolution as u32 > n_l {
        return None;
    }

    let Some(scale_factor) = pow2_u64(n_l - resolution as u32) else {
        return None;
    };
    let Some(x_step) = pow2_u64(ppx_u8 as u32) else {
        return None;
    };
    let Some(y_step) = pow2_u64(ppy_u8 as u32) else {
        return None;
    };

    let x_rsiz = component.size_info.horizontal_resolution as u64;
    let y_rsiz = component.size_info.vertical_resolution as u64;
    if x_rsiz == 0 || y_rsiz == 0 {
        return None;
    }

    let x_denom = x_rsiz.saturating_mul(scale_factor);
    let y_denom = y_rsiz.saturating_mul(scale_factor);
    if x_denom == 0 || y_denom == 0 {
        return None;
    }

    let ll_rect = res_data.tile_instance.resolution_transformed_rect;
    let trx0 = ll_rect.x0 as u64;
    let try0 = ll_rect.y0 as u64;

    let x_index_raw = floor_div_u64(ceil_div_u64(x, x_denom), x_step);
    let base_x = floor_div_u64(trx0, x_step);
    let x_index = x_index_raw.checked_sub(base_x)?;

    let y_index_raw = floor_div_u64(ceil_div_u64(y, y_denom), y_step);
    let base_y = floor_div_u64(try0, y_step);
    let y_index = y_index_raw.checked_sub(base_y)?;

    let precinct_width = res_data.precincts_wide as u64;
    if precinct_width == 0 {
        return None;
    }

    let precinct_index = x_index + precinct_width * y_index;
    precinct_index.try_into().ok()
}

fn ceil_div_u64(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }

    if numerator == 0 {
        0
    } else {
        (numerator - 1) / denominator + 1
    }
}

fn floor_div_u64(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        0
    } else {
        numerator / denominator
    }
}

fn pow2_u64(exp: u32) -> Option<u64> {
    if exp >= 64 { None } else { Some(1u64 << exp) }
}
