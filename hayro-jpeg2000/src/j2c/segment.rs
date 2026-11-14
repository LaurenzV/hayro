use alloc::boxed::Box;

use super::build::Segment;
use super::codestream::markers::{EPH, SOP};
use super::codestream::{ComponentInfo, Header};
use super::decode::DecompositionStorage;
use super::progression::ProgressionData;
use super::tile::{Tile, TilePart};
use crate::cancel::Cancel;
use crate::error::{Result, TileError, bail};
use crate::reader::BitReader;

pub(crate) const MAX_BITPLANE_COUNT: u8 = 32;

pub(crate) fn parse<'a, 'b>(
    tile: &'b Tile<'a>,
    mut progression_iterator: Box<dyn Iterator<Item = ProgressionData> + '_>,
    header: &Header,
    storage: &mut DecompositionStorage<'a>,
    cancel: Cancel<'_>,
) -> Result<()> {
    for tile_part in &tile.tile_parts {
        if parse_inner(
            tile_part.clone(),
            &mut progression_iterator,
            &tile.component_infos,
            storage,
            cancel,
        )?
        .is_none()
            && header.strict
        {
            bail!(TileError::Invalid);
        }
    }

    Ok(())
}

fn parse_inner<'a>(
    mut tile_part: TilePart<'a>,
    progression_iterator: &mut dyn Iterator<Item = ProgressionData>,
    component_infos: &[ComponentInfo],
    storage: &mut DecompositionStorage<'a>,
    cancel: Cancel<'_>,
) -> Result<Option<()>> {
    while !tile_part.header().at_end() {
        cancel.check()?;

        if parse_packet(
            &mut tile_part,
            progression_iterator,
            component_infos,
            storage,
        )
        .is_none()
        {
            return Ok(None);
        }
    }

    Ok(Some(()))
}

fn parse_packet<'a>(
    tile_part: &mut TilePart<'a>,
    progression_iterator: &mut dyn Iterator<Item = ProgressionData>,
    component_infos: &[ComponentInfo],
    storage: &mut DecompositionStorage<'a>,
) -> Option<()> {
    let progression_data = progression_iterator.next()?;
    let resolution = progression_data.resolution;
    let component_info = &component_infos[progression_data.component as usize];
    let tile_decompositions = &mut storage.tile_decompositions[progression_data.component as usize];
    let sub_band_iter = tile_decompositions.sub_band_iter(resolution, &storage.decompositions);

    let body_reader = tile_part.body();

    if component_info.coding_style.flags.may_use_sop_markers()
        && body_reader.peek_marker() == Some(SOP)
    {
        body_reader.read_marker().ok()?;
        body_reader.skip_bytes(4)?;
    }

    let header_reader = tile_part.header();

    let zero_length = header_reader.read_bits_with_stuffing(1)? == 0;

    if !zero_length {
        for sub_band in sub_band_iter.clone() {
            resolve_segments(
                sub_band,
                &progression_data,
                header_reader,
                storage,
                component_info,
            )?;
        }
    }

    header_reader.align();

    if component_info.coding_style.flags.uses_eph_marker()
        && header_reader.read_marker().ok()? != EPH
    {
        return None;
    }

    let body_reader = tile_part.body();

    if !zero_length {
        for sub_band in sub_band_iter {
            let sub_band = &mut storage.sub_bands[sub_band];
            let precinct = &mut storage.precincts[sub_band.precincts.clone()]
                [progression_data.precinct as usize];
            let code_blocks = &mut storage.code_blocks[precinct.code_blocks.clone()];

            for code_block in code_blocks {
                let layer = &mut storage.layers[code_block.layers.clone()]
                    [progression_data.layer_num as usize];

                if let Some(segments) = layer.segments.clone() {
                    let segments = &mut storage.segments[segments.clone()];

                    for segment in segments {
                        segment.data = body_reader.read_bytes(segment.data_length as usize)?;
                    }
                }
            }
        }
    }

    Some(())
}

fn resolve_segments(
    sub_band_dx: usize,
    progression_data: &ProgressionData,
    reader: &mut BitReader<'_>,
    storage: &mut DecompositionStorage<'_>,
    component_info: &ComponentInfo,
) -> Option<()> {
    const MAX_CODING_PASSES: u8 = 1 + 3 * (MAX_BITPLANE_COUNT - 1);

    let sub_band = &storage.sub_bands[sub_band_dx];
    let precincts = &mut storage.precincts[sub_band.precincts.clone()];
    let precinct = precincts.get_mut(progression_data.precinct as usize)?;
    let code_blocks = &mut storage.code_blocks[precinct.code_blocks.clone()];

    for code_block in code_blocks {
        let is_included = if code_block.has_been_included {
            reader.read_bits_with_stuffing(1)? == 1
        } else {
            precinct.code_inclusion_tree.read(
                code_block.x_idx,
                code_block.y_idx,
                reader,
                progression_data.layer_num as u32 + 1,
                &mut storage.tag_tree_nodes,
            )? <= progression_data.layer_num as u32
        };

        if !is_included {
            continue;
        }

        let layer =
            &mut storage.layers[code_block.layers.clone()][progression_data.layer_num as usize];

        let included_first_time = is_included && !code_block.has_been_included;

        if included_first_time {
            code_block.missing_bit_planes = precinct.zero_bitplane_tree.read(
                code_block.x_idx,
                code_block.y_idx,
                reader,
                u32::MAX,
                &mut storage.tag_tree_nodes,
            )? as u8;
        }

        code_block.has_been_included |= is_included;

        let added_coding_passes = if reader.peak_bits_with_stuffing(9) == Some(0x1ff) {
            reader.read_bits_with_stuffing(9)?;
            reader.read_bits_with_stuffing(7)? + 37
        } else if reader.peak_bits_with_stuffing(4) == Some(0x0f) {
            reader.read_bits_with_stuffing(4)?;
            reader.read_bits_with_stuffing(5)? + 6
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1110) {
            reader.read_bits_with_stuffing(4)?;
            5
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1101) {
            reader.read_bits_with_stuffing(4)?;
            4
        } else if reader.peak_bits_with_stuffing(4) == Some(0b1100) {
            reader.read_bits_with_stuffing(4)?;
            3
        } else if reader.peak_bits_with_stuffing(2) == Some(0b10) {
            reader.read_bits_with_stuffing(2)?;
            2
        } else if reader.peak_bits_with_stuffing(1) == Some(0) {
            reader.read_bits_with_stuffing(1)?;
            1
        } else {
            return None;
        } as u8;

        let mut k = 0;

        while reader.read_bits_with_stuffing(1)? == 1 {
            k += 1;
        }

        code_block.l_block += k;

        let previous_layers_passes = code_block.number_of_coding_passes;
        let cumulative_passes = previous_layers_passes.checked_add(added_coding_passes)?;

        if cumulative_passes > MAX_CODING_PASSES {
            return None;
        }

        let get_segment_idx = |pass_idx: u8| {
            if component_info.code_block_style().termination_on_each_pass {
                pass_idx
            } else if component_info
                .code_block_style()
                .selective_arithmetic_coding_bypass
            {
                segment_idx_for_bypass(pass_idx)
            } else {
                code_block.non_empty_layer_count
            }
        };

        let start = storage.segments.len();

        let mut push_segment = |segment: u8, coding_passes_for_segment: u8| {
            let length = {
                assert!(coding_passes_for_segment > 0);

                let length_bits = code_block.l_block + coding_passes_for_segment.ilog2();
                reader.read_bits_with_stuffing(length_bits as u8)
            }?;

            storage.segments.push(Segment {
                idx: segment,
                data_length: length,
                coding_pases: coding_passes_for_segment,
                data: &[],
            });

            Some(())
        };

        let mut last_segment = get_segment_idx(previous_layers_passes);
        let mut coding_passes_for_segment = 0;

        for coding_pass in previous_layers_passes..cumulative_passes {
            let segment = get_segment_idx(coding_pass);

            if segment != last_segment {
                push_segment(last_segment, coding_passes_for_segment)?;
                last_segment = segment;
                coding_passes_for_segment = 1;
            } else {
                coding_passes_for_segment += 1;
            }
        }

        if coding_passes_for_segment > 0 {
            push_segment(last_segment, coding_passes_for_segment)?;
        }

        let end = storage.segments.len();
        layer.segments = Some(start..end);
        code_block.number_of_coding_passes += added_coding_passes;
        code_block.non_empty_layer_count += 1;
    }

    Some(())
}

fn segment_idx_for_bypass(pass_idx: u8) -> u8 {
    if pass_idx < 10 {
        0
    } else {
        1 + (2 * ((pass_idx - 10) / 3)) + (if ((pass_idx - 10) % 3) == 2 { 1 } else { 0 })
    }
}
