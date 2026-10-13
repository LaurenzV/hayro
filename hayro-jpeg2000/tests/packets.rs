//! Packets and progression orders (B.9-B.12).

use crate::*;

#[test]
fn progression_lrcp() {
    check("packets_progression_lrcp", reference("main_rgba"));
}

#[test]
fn progression_rlcp() {
    check("packets_progression_rlcp", reference("main_rgba"));
}

#[test]
fn progression_rpcl() {
    check("packets_progression_rpcl", reference("main_rgba"));
}

#[test]
fn progression_pcrl() {
    check("packets_progression_pcrl", reference("main_rgba"));
}

#[test]
fn progression_cprl() {
    check("packets_progression_cprl", reference("main_rgba"));
}

#[test]
fn sop() {
    check("packets_sop", reference("main_r"));
}

#[test]
fn eph() {
    check("packets_eph", reference("main_r"));
}

#[test]
fn sop_eph() {
    check("packets_sop_eph", reference("main_r"));
}

#[test]
fn layers_many() {
    check("packets_layers_many", reference("main_r"));
}

#[test]
fn late_inclusion() {
    check("packets_late_inclusion", reference("main_r"));
}

#[test]
fn lblock_large() {
    check(
        "packets_lblock_large",
        reference("stress_16bit_crop_64_0_64_64"),
    );
}

#[test]
fn passes_codeword_long() {
    check(
        "packets_passes_codeword_long",
        reference("stress_16bit_crop_64_0_64_64"),
    );
}

#[test]
fn sop_missing() {
    check("packets_sop_missing", reference("main_r"));
}

#[test]
fn ppm() {
    check("packets_ppm", reference("main_rgb"));
}

#[test]
fn ppm_sop_eph() {
    check("packets_ppm_sop_eph", reference("main_rgb"));
}

#[test]
fn ppm_many_segments() {
    check("packets_ppm_many_segments", reference("main_rgb"));
}

#[test]
fn ppm_spanning_segments() {
    check("packets_ppm_spanning_segments", reference("main_r"));
}

#[test]
fn ppm_many_tiles() {
    check("packets_ppm_many_tiles", reference("main_r"));
}

#[test]
fn ppt() {
    check("packets_ppt", reference("main_rgb"));
}

#[test]
fn ppt_many_segments() {
    check("packets_ppt_many_segments", reference("main_rgb"));
}

#[test]
fn ppt_sop_eph_tiny_tiles() {
    check(
        "packets_ppt_sop_eph_tiny_tiles",
        reference("main_rgb_crop_50_40_12_12"),
    );
}

#[test]
fn poc_lrcp() {
    check("packets_poc_lrcp", reference("main_rgb"));
}

#[test]
fn poc_rlcp() {
    check("packets_poc_rlcp", reference("main_rgb"));
}

#[test]
fn poc_rpcl() {
    check("packets_poc_rpcl", reference("main_rgb"));
}

#[test]
fn poc_pcrl() {
    check("packets_poc_pcrl", reference("main_rgb"));
}

#[test]
fn poc_cprl() {
    check("packets_poc_cprl", reference("main_rgb"));
}

#[test]
fn poc_main_header() {
    check("packets_poc_main_header", reference("main_rgb"));
}

#[test]
fn poc_main_header_tile_override() {
    check(
        "packets_poc_main_header_tile_override",
        reference("main_rgb"),
    );
}

#[test]
fn poc_per_tile() {
    check("packets_poc_per_tile", reference("main_rgb"));
}

#[test]
fn poc_main_header_ranges() {
    check("packets_poc_main_header_ranges", reference("main_rgb"));
}

#[test]
fn poc_clamped_values() {
    check("packets_poc_clamped_values", reference("main_rgb"));
}

#[test]
fn poc_subsampled() {
    check(
        "packets_poc_subsampled",
        reference("main_rgb_sub_1x1_2x2_1x2"),
    );
}

#[test]
fn poc_offset() {
    check("packets_poc_offset", reference("main_rgb"));
}

#[test]
fn poc_tile_parts() {
    check("packets_poc_tile_parts", reference("main_rgb"));
}

#[test]
fn mixed_levels_lrcp() {
    check("packets_mixed_levels_lrcp", reference("main_rgb"));
}

#[test]
fn mixed_levels_rlcp() {
    check("packets_mixed_levels_rlcp", reference("main_rgb"));
}

#[test]
fn mixed_levels_rpcl() {
    check("packets_mixed_levels_rpcl", reference("main_rgb"));
}

#[test]
fn mixed_levels_pcrl() {
    check("packets_mixed_levels_pcrl", reference("main_rgb"));
}

#[test]
fn mixed_levels_cprl() {
    check("packets_mixed_levels_cprl", reference("main_rgb"));
}

#[test]
fn empty_packets() {
    check("packets_empty_packets", reference("main_rgba"));
}

#[test]
fn eph_empty_packets() {
    check("packets_eph_empty_packets", reference("main_rgba"));
}

#[test]
fn invalid_stuff_bit() {
    check(
        "packets_invalid_stuff_bit",
        reference("packets_invalid_stuff_bit"),
    );
}
