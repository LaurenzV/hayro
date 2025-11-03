use crate::codestream::ComponentInfo;
use crate::tile::{IntRect, Tile, TileInstance};

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
            tile,
            max_resolutions,
        }
    }
}

pub(crate) fn build_layer_resolution_component_position_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();

    for layer in 0..input.layers {
        for resolution in 0..input.max_resolutions {
            let resolution = resolution;
            let tile_instances = tile_instances_for_resolution(input, resolution);

            for (component_idx, tile_instance_opt) in tile_instances.into_iter().enumerate() {
                let Some(tile_instance) = tile_instance_opt else {
                    continue;
                };

                let precinct_count = tile_instance.num_precincts();
                if precinct_count == 0 {
                    continue;
                }

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

pub(crate) fn build_resolution_layer_component_position_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();

    for resolution in 0..input.max_resolutions {
        let resolution = resolution;
        let tile_instances = tile_instances_for_resolution(input, resolution);

        for layer in 0..input.layers {
            for (component_idx, tile_instance_opt) in tile_instances.iter().enumerate() {
                let Some(tile_instance) = tile_instance_opt else {
                    continue;
                };

                let precinct_count = tile_instance.num_precincts();
                if precinct_count == 0 {
                    continue;
                }

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

pub(crate) fn build_resolution_position_component_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let tile_rect = input.tile.rect;

    for resolution in 0..input.max_resolutions {
        let tile_instances = tile_instances_for_resolution(input, resolution);

        for y in tile_rect.y0..tile_rect.y1 {
            for x in tile_rect.x0..tile_rect.x1 {
                for (component_idx, tile_instance_opt) in tile_instances.iter().enumerate() {
                    let Some(tile_instance) = tile_instance_opt else {
                        continue;
                    };
                    let component_info = &input.tile.component_info[component_idx];

                    if let Some(precinct) =
                        find_precinct_index(tile_instance, component_info, tile_rect, x, y)
                    {
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

    sequence
}

pub(crate) fn build_position_component_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let tile_rect = input.tile.rect;

    for y in tile_rect.y0..tile_rect.y1 {
        for x in tile_rect.x0..tile_rect.x1 {
            for (component_idx, component_info) in input.tile.component_info.iter().enumerate() {
                let num_resolution_levels = component_info
                    .coding_style_parameters
                    .parameters
                    .num_resolution_levels;

                for resolution in 0..num_resolution_levels {
                    let tile_instance = component_info.tile_instance(input.tile, resolution);

                    if let Some(precinct) =
                        find_precinct_index(&tile_instance, component_info, tile_rect, x, y)
                    {
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

    sequence
}

pub(crate) fn build_component_position_resolution_layer_sequence(
    input: &IteratorInput<'_>,
) -> Vec<ProgressionData> {
    let mut sequence = Vec::new();
    let tile_rect = input.tile.rect;

    for (component_idx, component_info) in input.tile.component_info.iter().enumerate() {
        let num_resolution_levels = component_info
            .coding_style_parameters
            .parameters
            .num_resolution_levels;

        for y in tile_rect.y0..tile_rect.y1 {
            for x in tile_rect.x0..tile_rect.x1 {
                for resolution in 0..num_resolution_levels {
                    let tile_instance = component_info.tile_instance(input.tile, resolution);

                    if let Some(precinct) =
                        find_precinct_index(&tile_instance, component_info, tile_rect, x, y)
                    {
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

    sequence
}

fn tile_instances_for_resolution<'a>(
    input: &'a IteratorInput<'a>,
    resolution: u16,
) -> Vec<Option<TileInstance<'a>>> {
    input
        .tile
        .component_info
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

fn find_precinct_index(
    tile_instance: &TileInstance,
    component_info: &ComponentInfo,
    tile_rect: IntRect,
    x: u32,
    y: u32,
) -> Option<u32> {
    if tile_instance.num_precincts() == 0 {
        return None;
    }

    let num_decomposition_levels = component_info
        .coding_style_parameters
        .parameters
        .num_decomposition_levels as u32;
    let resolution = tile_instance.resolution as u32;
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

    let y_stride_shift = tile_instance.ppy() as u32 + base_shift;
    let x_stride_shift = tile_instance.ppx() as u32 + base_shift;
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
    let try0 = tile_instance.resolution_transformed_rect.y0 as u64;
    let trx0 = tile_instance.resolution_transformed_rect.x0 as u64;

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

    let precinct_x_scale = 1u64 << (tile_instance.ppx() as u32);
    let precinct_y_scale = 1u64 << (tile_instance.ppy() as u32);

    let p1 = x_val.div_ceil(horizontal_denom) / precinct_x_scale;
    let p2 = trx0 / precinct_x_scale;
    let diff_x = p1.checked_sub(p2)?;

    let p4 = y_val.div_ceil(vertical_denom) / precinct_y_scale;
    let p5 = try0 / precinct_y_scale;
    let diff_y = p4.checked_sub(p5)?;

    let precincts_wide = tile_instance.num_precincts_x() as u64;
    if precincts_wide == 0 {
        return None;
    }

    let precinct = diff_x + precincts_wide * diff_y;
    if precinct >= tile_instance.num_precincts() as u64 {
        return None;
    }

    precinct.try_into().ok()
}
