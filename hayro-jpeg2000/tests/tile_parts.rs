//! Tile-parts (A.4).

use crate::*;

#[test]
fn split_resolution() {
    check("tile_parts_split_resolution", reference("main_r"));
}

#[test]
fn split_layer() {
    check("tile_parts_split_layer", reference("main_r"));
}

#[test]
fn split_component() {
    check("tile_parts_split_component", reference("main_rgb"));
}

#[test]
fn interleaved() {
    check("tile_parts_interleaved", reference("main_r"));
}

#[test]
fn tnsot_zero() {
    check("tile_parts_tnsot_zero", reference("main_r"));
}

#[test]
fn tnsot_too_small() {
    check("tile_parts_tnsot_too_small", reference("main_r"));
}

#[test]
fn index_overflow() {
    check("tile_parts_index_overflow", reference("main_r"));
}

#[test]
fn psot_zero() {
    check("tile_parts_psot_zero", reference("main_r"));
}

#[test]
fn psot_zero_padding() {
    check("tile_parts_psot_zero_padding", reference("main_r"));
}

#[test]
fn missing_eoc() {
    check("tile_parts_missing_eoc", reference("main_r"));
}

#[test]
fn bytes_after_eoc() {
    check("tile_parts_bytes_after_eoc", reference("main_r"));
}

#[test]
fn garbage_before_eoc() {
    check("tile_parts_garbage_before_eoc", reference("main_r"));
}

#[test]
fn truncated_end() {
    check("tile_parts_truncated_end", reference("main_r"));
}

#[test]
fn trailing_packet_data() {
    check("tile_parts_trailing_packet_data", reference("main_r"));
}

#[test]
fn truncated_layers() {
    check(
        "tile_parts_truncated_layers",
        reference("tile_parts_truncated_layers"),
    );
}

#[test]
fn sot_only_tile_part() {
    check("tile_parts_sot_only_tile_part", reference("main_r"));
}

#[test]
fn cod_qcd_in_later_tile_part() {
    check("tile_parts_cod_qcd_in_later_tile_part", reference("main_r"));
}

#[test]
fn truncated_in_packet_header() {
    check(
        "tile_parts_truncated_in_packet_header",
        reference("tile_parts_truncated_in_packet_header"),
    );
}

#[test]
fn truncated_in_packet_body() {
    check(
        "tile_parts_truncated_in_packet_body",
        reference("tile_parts_truncated_in_packet_body"),
    );
}
