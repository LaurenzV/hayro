//! Invalid streams that are decoded leniently, like most other decoders do.

use crate::check_lenient;

#[test]
fn intermediate_page_refinement() {
    check_lenient("lenient_intermediate_page_refinement", "truchet");
}

#[test]
fn unknown_length_intermediate() {
    check_lenient("lenient_unknown_length_intermediate", "truchet");
}

#[test]
fn missing_reference() {
    check_lenient("lenient_missing_reference", "honeycomb");
}

#[test]
fn refinement_of_immediate_region() {
    check_lenient("lenient_refinement_of_immediate_region", "truchet");
}

#[test]
fn necessary_extension() {
    check_lenient("lenient_necessary_extension", "honeycomb");
}

#[test]
fn reserved_region_flags() {
    check_lenient("lenient_reserved_region_flags", "truchet");
}

#[test]
fn reserved_segment_type() {
    check_lenient("lenient_reserved_segment_type", "truchet");
}

#[test]
fn trailing_bytes() {
    check_lenient("lenient_trailing_bytes", "honeycomb");
}

#[test]
fn truncated_segment() {
    check_lenient("lenient_truncated_segment", "truchet");
}

#[test]
fn missing_final_oob() {
    check_lenient("lenient_missing_final_oob", "honeycomb");
}

#[test]
fn missing_final_oob_huffman() {
    check_lenient("lenient_missing_final_oob_huffman", "honeycomb");
}

#[test]
fn table_past_hthigh() {
    check_lenient("lenient_table_past_hthigh", "honeycomb");
}

#[test]
fn gray_value_out_of_range() {
    check_lenient("lenient_gray_value_out_of_range", "maze");
}

#[test]
fn mmr_rows_missing() {
    check_lenient("lenient_mmr_rows_missing", "truchet");
}

#[test]
fn failing_region() {
    check_lenient("lenient_failing_region", "honeycomb");
}

#[test]
fn symbol_out_of_range() {
    check_lenient("lenient_symbol_out_of_range", "honeycomb");
}

#[test]
fn colour_region() {
    check_lenient("lenient_colour_region", "honeycomb");
}
