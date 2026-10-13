//! Main header syntax and image geometry (Annex A, B.2-B.5).

use crate::*;

#[test]
fn gray() {
    check("codestream_gray", reference("main_r"));
}

#[test]
fn odd_size() {
    check("codestream_odd_size", reference("main_r_crop_5_3_61_47"));
}

#[test]
fn tiny() {
    check("codestream_tiny", reference("main_r_crop_60_40_3_5"));
}

#[test]
fn single_row() {
    check("codestream_single_row", reference("main_r_crop_0_45_128_1"));
}

#[test]
fn single_row_odd_offset() {
    check(
        "codestream_single_row_odd_offset",
        reference("main_r_crop_0_45_128_1"),
    );
}

#[test]
fn tall_narrow_subsampled_offset() {
    check(
        "codestream_tall_narrow_subsampled_offset",
        reference("main_rg_crop_40_0_5_96_sub_1x1_2x1_at_123_0"),
    );
}

#[test]
fn offset() {
    check("codestream_offset", reference("main_r"));
}

#[test]
fn offset_large() {
    check("codestream_offset_large", reference("main_r"));
}

#[test]
fn tiles_partial() {
    check("codestream_tiles_partial", reference("main_rgb"));
}

#[test]
fn tiles_offset_many() {
    check("codestream_tiles_offset_many", reference("main_r"));
}

#[test]
fn tile_larger_than_image() {
    check("codestream_tile_larger_than_image", reference("main_rgb"));
}

#[test]
fn precision_4() {
    check("codestream_precision_4", reference("main_r_4bit"));
}

#[test]
fn precision_10() {
    check("codestream_precision_10", reference("main_r_10bit"));
}

#[test]
fn precision_12() {
    check("codestream_precision_12", reference("main_r_12bit"));
}

#[test]
fn precision_16() {
    check("codestream_precision_16", reference("main_r_16bit"));
}

#[test]
fn precision_mixed() {
    check(
        "codestream_precision_mixed",
        reference("main_rgba_5_5_5_1bit"),
    );
}

#[test]
fn signed_4() {
    check("codestream_signed_4", reference("main_r_4bit"));
}

#[test]
fn signed_12() {
    check("codestream_signed_12", reference("main_r_12bit"));
}

#[test]
fn signed_16() {
    check("codestream_signed_16", reference("main_r_16bit"));
}

#[test]
fn subsampling_2x1() {
    check(
        "codestream_subsampling_2x1",
        reference("main_rg_sub_1x1_2x1"),
    );
}

#[test]
fn subsampling_1x2() {
    check(
        "codestream_subsampling_1x2",
        reference("main_rg_sub_1x1_1x2"),
    );
}

#[test]
fn subsampling_2x2() {
    check(
        "codestream_subsampling_2x2",
        reference("main_rg_sub_1x1_2x2"),
    );
}

#[test]
fn subsampling_4x1() {
    check(
        "codestream_subsampling_4x1",
        reference("main_rg_sub_1x1_4x1"),
    );
}

#[test]
fn subsampling_4x4() {
    check(
        "codestream_subsampling_4x4",
        reference("main_rg_sub_1x1_4x4"),
    );
}

#[test]
fn subsampling_odd_factors() {
    check(
        "codestream_subsampling_odd_factors",
        reference("main_rgb_sub_1x1_11x5_13x6"),
    );
}

#[test]
fn subsampling_per_component() {
    check(
        "codestream_subsampling_per_component",
        reference("main_rgba_sub_1x1_1x2_2x1_2x2"),
    );
}

#[test]
fn subsampling_all_components() {
    check(
        "codestream_subsampling_all_components",
        reference("main_rgb_sub_2x2_2x2_2x2"),
    );
}

#[test]
fn subsampling_all_odd() {
    check(
        "codestream_subsampling_all_odd",
        reference("main_rgb_sub_17x7_17x7_17x7"),
    );
}

#[test]
fn subsampling_with_offset() {
    check(
        "codestream_subsampling_with_offset",
        reference("main_r_sub_5x3_at_1_160"),
    );
}

#[test]
fn levels_0() {
    check("codestream_levels_0", reference("main_r"));
}

#[test]
fn levels_1() {
    check("codestream_levels_1", reference("main_r"));
}

#[test]
fn levels_8() {
    check("codestream_levels_8", reference("main_r"));
}

#[test]
fn levels_more_than_size() {
    check(
        "codestream_levels_more_than_size",
        reference("main_r_crop_10_10_17_9"),
    );
}

#[test]
fn codeblock_4x16() {
    check("codestream_codeblock_4x16", reference("main_r"));
}

#[test]
fn codeblock_8x64() {
    check("codestream_codeblock_8x64", reference("main_r"));
}

#[test]
fn codeblock_64x32() {
    check("codestream_codeblock_64x32", reference("main_r"));
}

#[test]
fn codeblock_32x32() {
    check("codestream_codeblock_32x32", reference("main_r"));
}

#[test]
fn codeblock_128x32() {
    check("codestream_codeblock_128x32", reference("main_r"));
}

#[test]
fn codeblock_precinct_limited() {
    check("codestream_codeblock_precinct_limited", reference("main_r"));
}

#[test]
fn precincts() {
    check("codestream_precincts", reference("main_rgb"));
}

#[test]
fn precincts_non_square() {
    check("codestream_precincts_non_square", reference("main_r"));
}

#[test]
fn precincts_1x1() {
    check(
        "codestream_precincts_1x1",
        reference("main_r_crop_40_30_8_12"),
    );
}

#[test]
fn guard_bits_zero() {
    check("codestream_guard_bits_zero", reference("main_r"));
}

#[test]
fn guard_bits_high() {
    check("codestream_guard_bits_high", reference("main_r"));
}

#[test]
fn tlm() {
    check("codestream_tlm", reference("main_rgb"));
}

#[test]
fn plt() {
    check("codestream_plt", reference("main_rgb"));
}

#[test]
fn components_5() {
    check_components("codestream_components_5", reference("main_rgbar"));
}

#[test]
fn components_5_subsampled() {
    check_components(
        "codestream_components_5_subsampled",
        reference("main_rgbar_sub_1x1_1x1_1x1_2x2_2x2"),
    );
}

#[test]
fn crg() {
    check("codestream_crg", reference("main_rgb"));
}

#[test]
fn reserved_marker() {
    check("codestream_reserved_marker", reference("main_r"));
}

#[test]
fn tile_size_huge() {
    check("codestream_tile_size_huge", reference("main_r"));
}

#[test]
fn tile_cod_progression() {
    check("codestream_tile_cod_progression", reference("main_rgb"));
}

#[test]
fn tile_cod_layers() {
    check("codestream_tile_cod_layers", reference("main_r"));
}

#[test]
fn tile_cod_fewer_levels() {
    check("codestream_tile_cod_fewer_levels", reference("main_r"));
}

#[test]
fn tile_qcd() {
    check("codestream_tile_qcd", reference("main_r"));
}

#[test]
fn levels_per_component() {
    check("codestream_levels_per_component", reference("main_rgb"));
}

#[test]
fn coc_style_per_component() {
    check("codestream_coc_style_per_component", reference("main_rgb"));
}

#[test]
fn qcc_per_component() {
    check("codestream_qcc_per_component", reference("main_rgb"));
}

#[test]
fn tile_coc_qcc() {
    check("codestream_tile_coc_qcc", reference("main_rgb"));
}

#[test]
fn com_in_tile_part() {
    check("codestream_com_in_tile_part", reference("main_r"));
}

#[test]
fn coc_wavelet_per_component() {
    check(
        "codestream_coc_wavelet_per_component",
        reference("main_rgb").tolerance(1),
    );
}
