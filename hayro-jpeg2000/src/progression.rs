use crate::codestream::{ComponentInfo, ProgressionOrder};
use crate::tile::{IntRect, Tile, TileInstance};
use std::cmp::Ordering;

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
    max_resolutions: u16,
}

impl<'a> IteratorInput<'a> {
    pub(crate) fn new(
        tile: &'a Tile<'a>,
        component_infos: &'a [ComponentInfo],
        layers: u16,
    ) -> Self {
        let max_resolutions = component_infos
            .iter()
            .map(|c| c.coding_style_parameters.parameters.num_resolution_levels)
            .max()
            .unwrap_or(0);

        Self {
            layers,
            component_infos,
            tile,
            max_resolutions,
        }
    }
}

pub(crate) fn build_progression_sequence<'a>(
    input: &IteratorInput<'a>,
    order: ProgressionOrder,
) -> Vec<ProgressionData> {
    match order {
        ProgressionOrder::LayerResolutionComponentPosition => {
            build_layer_resolution_component_position_sequence(input)
        }
        ProgressionOrder::ResolutionLayerComponentPosition => {
            build_resolution_layer_component_position_sequence(input)
        }
        ProgressionOrder::ResolutionPositionComponentLayer => {
            build_resolution_position_component_layer_sequence(input)
        }
        ProgressionOrder::PositionComponentResolutionLayer => {
            build_position_component_resolution_layer_sequence(input)
        }
        ProgressionOrder::ComponentPositionResolutionLayer => {
            build_component_position_resolution_layer_sequence(input)
        }
    }
}

fn build_layer_resolution_component_position_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();

    for layer in 0..input.layers {
        for resolution in 0..input.max_resolutions {
            let resolution = resolution as u16;
            let tile_instances = tile_instances_for_resolution(input, resolution);

            for (component_idx, tile_instance_opt) in tile_instances.into_iter().enumerate() {
                let Some(tile_instance) = tile_instance_opt else {
                    continue;
                };
                let precinct_count = tile_instance.num_precincts();
                for precinct in 0..precinct_count {
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

    sequence
}

fn build_resolution_layer_component_position_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();

    for resolution in 0..input.max_resolutions {
        let resolution = resolution as u16;
        let tile_instances = tile_instances_for_resolution(input, resolution);

        for layer in 0..input.layers {
            for (component_idx, tile_instance_opt) in tile_instances.iter().enumerate() {
                let Some(tile_instance) = tile_instance_opt else {
                    continue;
                };
                let precinct_count = tile_instance.num_precincts();
                for precinct in 0..precinct_count {
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

    sequence
}

fn build_resolution_position_component_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();

    for resolution in 0..input.max_resolutions {
        let resolution = resolution as u16;
        let mut entries = build_entries_for_resolution(input, resolution);
        emit_progression(
            &mut entries,
            input.layers,
            &mut sequence,
            compare_resolution_position_component_layer,
        );
    }

    sequence
}

fn build_position_component_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let mut entries = build_entries_for_all_components(input);

    emit_progression(
        &mut entries,
        input.layers,
        &mut sequence,
        compare_position_component_resolution_layer,
    );

    sequence
}

fn build_component_position_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let mut entries = build_entries_for_all_components(input);

    emit_progression(
        &mut entries,
        input.layers,
        &mut sequence,
        compare_component_position_resolution_layer,
    );

    sequence
}

fn tile_instances_for_resolution<'a>(
    input: &'a IteratorInput<'a>,
    resolution: u16,
) -> Vec<Option<TileInstance<'a>>> {
    input
        .component_infos
        .iter()
        .map(|component_info| {
            if resolution
                < component_info
                    .coding_style_parameters
                    .parameters
                    .num_resolution_levels
            {
                Some(component_info.tile_instance(input.tile, resolution))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct PrecinctPosition {
    precinct: u32,
    x: u32,
    y: u32,
}

struct PrecinctState {
    tile_rect: IntRect,
    num_decomposition_levels: u32,
    x: u32,
    y: u32,
}

impl PrecinctState {
    fn new(tile_rect: IntRect, num_decomposition_levels: u32) -> Self {
        Self {
            tile_rect,
            num_decomposition_levels,
            x: tile_rect.x0,
            y: tile_rect.y0,
        }
    }

    fn next(
        &mut self,
        tile_instance: &TileInstance,
        component_info: &ComponentInfo,
    ) -> Option<PrecinctPosition> {
        if tile_instance.num_precincts() == 0
            || self.tile_rect.x0 >= self.tile_rect.x1
            || self.tile_rect.y0 >= self.tile_rect.y1
        {
            return None;
        }

        if self.y < self.tile_rect.y0 {
            self.y = self.tile_rect.y0;
        }
        if self.x < self.tile_rect.x0 {
            self.x = self.tile_rect.x0;
        }

        loop {
            if self.y >= self.tile_rect.y1 {
                return None;
            }

            if self.matches(tile_instance, component_info) {
                if let Some(precinct) = self.next_precinct(tile_instance, component_info) {
                    let position = PrecinctPosition {
                        precinct,
                        x: self.x,
                        y: self.y,
                    };

                    if self.advance_x() {
                        self.advance_y();
                    }

                    return Some(position);
                }
            }

            if self.advance_x() && self.advance_y() {
                return None;
            }
        }
    }

    fn matches(&self, tile_instance: &TileInstance, component_info: &ComponentInfo) -> bool {
        let n_l = self.num_decomposition_levels;
        let resolution = tile_instance.resolution as u32;
        if resolution > n_l {
            return false;
        }

        let vertical_resolution = component_info.size_info.vertical_resolution as u32;
        let horizontal_resolution = component_info.size_info.horizontal_resolution as u32;
        if vertical_resolution == 0 || horizontal_resolution == 0 {
            return false;
        }

        let base_exponent = match n_l.checked_sub(resolution) {
            Some(value) => value,
            None => return false,
        };

        let scale_factor = match pow2_u64(base_exponent) {
            Some(value) => value,
            None => return false,
        };

        let stride_y_exponent = match (tile_instance.ppy() as u32).checked_add(base_exponent) {
            Some(value) => value,
            None => return false,
        };
        let stride_x_exponent = match (tile_instance.ppx() as u32).checked_add(base_exponent) {
            Some(value) => value,
            None => return false,
        };

        let vertical_stride = match pow2_u64(stride_y_exponent) {
            Some(value) => vertical_resolution as u64 * value,
            None => return false,
        };
        let horizontal_stride = match pow2_u64(stride_x_exponent) {
            Some(value) => horizontal_resolution as u64 * value,
            None => return false,
        };

        if vertical_stride == 0 || horizontal_stride == 0 {
            return false;
        }

        let y_val = self.y as u64;
        let x_val = self.x as u64;
        let ty0 = self.tile_rect.y0 as u64;
        let tx0 = self.tile_rect.x0 as u64;
        let try0 = tile_instance.resolution_transformed_rect.y0 as u64;
        let trx0 = tile_instance.resolution_transformed_rect.x0 as u64;

        let matches_vertical = (y_val % vertical_stride == 0)
            || (y_val == ty0 && (try0 * scale_factor) % vertical_stride != 0);

        if !matches_vertical {
            return false;
        }

        (x_val % horizontal_stride == 0)
            || (x_val == tx0 && (trx0 * scale_factor) % horizontal_stride != 0)
    }

    fn next_precinct(
        &self,
        tile_instance: &TileInstance,
        component_info: &ComponentInfo,
    ) -> Option<u32> {
        let n_l = self.num_decomposition_levels;
        let resolution = tile_instance.resolution as u32;
        if resolution > n_l {
            return None;
        }

        let base_exponent = n_l.checked_sub(resolution)?;
        let scale_factor = pow2_u64(base_exponent)?;

        let horizontal_resolution = component_info.size_info.horizontal_resolution as u32;
        let vertical_resolution = component_info.size_info.vertical_resolution as u32;
        if horizontal_resolution == 0 || vertical_resolution == 0 {
            return None;
        }

        let denom_x = horizontal_resolution as u64 * scale_factor;
        let denom_y = vertical_resolution as u64 * scale_factor;
        if denom_x == 0 || denom_y == 0 {
            return None;
        }

        let ppx_factor = pow2_u64(tile_instance.ppx() as u32)?;
        let ppy_factor = pow2_u64(tile_instance.ppy() as u32)?;

        let p1 = ceil_div_u64(self.x as u64, denom_x) / ppx_factor;
        let p2 = (tile_instance.resolution_transformed_rect.x0 as u64) / ppx_factor;
        let diff_x = p1.checked_sub(p2)?;

        let p4 = ceil_div_u64(self.y as u64, denom_y) / ppy_factor;
        let p5 = (tile_instance.resolution_transformed_rect.y0 as u64) / ppy_factor;
        let diff_y = p4.checked_sub(p5)?;

        let precincts_wide = tile_instance.num_precincts_x() as u64;
        if precincts_wide == 0 {
            return None;
        }

        let precinct_index = diff_x + precincts_wide * diff_y;
        precinct_index.try_into().ok()
    }

    fn advance_x(&mut self) -> bool {
        if self.tile_rect.x0 >= self.tile_rect.x1 {
            return true;
        }

        if self.x + 1 >= self.tile_rect.x1 {
            self.x = self.tile_rect.x0;
            true
        } else {
            self.x += 1;
            false
        }
    }

    fn advance_y(&mut self) -> bool {
        if self.tile_rect.y0 >= self.tile_rect.y1 {
            self.y = self.tile_rect.y1;
            return true;
        }

        if self.y + 1 >= self.tile_rect.y1 {
            self.y = self.tile_rect.y1;
            true
        } else {
            self.y += 1;
            false
        }
    }
}

struct ProgressionEntry<'a> {
    component_idx: usize,
    resolution: u16,
    tile_instance: TileInstance<'a>,
    state: PrecinctState,
    current: Option<PrecinctPosition>,
}

fn build_entries_for_resolution<'a>(
    input: &'a IteratorInput<'a>,
    resolution: u16,
) -> Vec<ProgressionEntry<'a>> {
    let tile_rect = input.tile.rect;
    let mut entries = Vec::new();

    for (component_idx, component_info) in input.component_infos.iter().enumerate() {
        if resolution
            >= component_info
                .coding_style_parameters
                .parameters
                .num_resolution_levels
        {
            continue;
        }

        let tile_instance = component_info.tile_instance(input.tile, resolution);
        if tile_instance.num_precincts() == 0 {
            continue;
        }

        let mut state = PrecinctState::new(
            tile_rect,
            component_info
                .coding_style_parameters
                .parameters
                .num_decomposition_levels as u32,
        );
        let current = state.next(&tile_instance, component_info);

        if current.is_some() {
            entries.push(ProgressionEntry {
                component_idx,
                resolution,
                tile_instance,
                state,
                current,
            });
        }
    }

    entries
}

fn build_entries_for_all_components<'a>(input: &'a IteratorInput<'a>) -> Vec<ProgressionEntry<'a>> {
    let tile_rect = input.tile.rect;
    let mut entries = Vec::new();

    for (component_idx, component_info) in input.component_infos.iter().enumerate() {
        let num_resolution_levels = component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels;

        for resolution in 0..num_resolution_levels {
            let tile_instance = component_info.tile_instance(input.tile, resolution);
            if tile_instance.num_precincts() == 0 {
                continue;
            }

            let mut state = PrecinctState::new(
                tile_rect,
                component_info
                    .coding_style_parameters
                    .parameters
                    .num_decomposition_levels as u32,
            );
            let current = state.next(&tile_instance, component_info);

            if current.is_some() {
                entries.push(ProgressionEntry {
                    component_idx,
                    resolution,
                    tile_instance,
                    state,
                    current,
                });
            }
        }
    }

    entries
}

fn emit_progression<'a>(
    entries: &mut [ProgressionEntry<'a>],
    layers: u16,
    sequence: &mut Vec<ProgressionData>,
    compare: impl Fn(
        &ProgressionEntry<'a>,
        &PrecinctPosition,
        &ProgressionEntry<'a>,
        &PrecinctPosition,
    ) -> Ordering,
) {
    loop {
        let mut best_index: Option<usize> = None;

        for (idx, entry) in entries.iter().enumerate() {
            let Some(position) = entry.current.as_ref() else {
                continue;
            };

            best_index = Some(match best_index {
                None => idx,
                Some(current_best) => {
                    let best_entry = &entries[current_best];
                    let best_position = best_entry.current.as_ref().unwrap();
                    match compare(entry, position, best_entry, best_position) {
                        Ordering::Less => idx,
                        Ordering::Equal => current_best,
                        Ordering::Greater => current_best,
                    }
                }
            });
        }

        let Some(best_idx) = best_index else {
            break;
        };

        let (_before, after) = entries.split_at_mut(best_idx);
        let entry = &mut after[0];
        let position = entry.current.expect("entry must have current value");

        for layer in 0..layers {
            sequence.push(ProgressionData {
                layer_num: layer,
                resolution: entry.resolution,
                component: entry.component_idx as u8,
                precinct: position.precinct,
            });
        }

        entry.current = entry
            .state
            .next(&entry.tile_instance, entry.tile_instance.component_info);
    }
}

fn compare_resolution_position_component_layer(
    left_entry: &ProgressionEntry,
    left_pos: &PrecinctPosition,
    right_entry: &ProgressionEntry,
    right_pos: &PrecinctPosition,
) -> Ordering {
    left_pos
        .y
        .cmp(&right_pos.y)
        .then_with(|| left_pos.x.cmp(&right_pos.x))
        .then_with(|| left_entry.component_idx.cmp(&right_entry.component_idx))
}

fn compare_position_component_resolution_layer(
    left_entry: &ProgressionEntry,
    left_pos: &PrecinctPosition,
    right_entry: &ProgressionEntry,
    right_pos: &PrecinctPosition,
) -> Ordering {
    left_pos
        .y
        .cmp(&right_pos.y)
        .then_with(|| left_pos.x.cmp(&right_pos.x))
        .then_with(|| left_entry.component_idx.cmp(&right_entry.component_idx))
        .then_with(|| left_entry.resolution.cmp(&right_entry.resolution))
}

fn compare_component_position_resolution_layer(
    left_entry: &ProgressionEntry,
    left_pos: &PrecinctPosition,
    right_entry: &ProgressionEntry,
    right_pos: &PrecinctPosition,
) -> Ordering {
    left_entry
        .component_idx
        .cmp(&right_entry.component_idx)
        .then_with(|| left_pos.y.cmp(&right_pos.y))
        .then_with(|| left_pos.x.cmp(&right_pos.x))
        .then_with(|| left_entry.resolution.cmp(&right_entry.resolution))
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

fn pow2_u64(exp: u32) -> Option<u64> {
    if exp >= 64 { None } else { Some(1u64 << exp) }
}
