//! Invalid streams that are decoded leniently, like most other decoders do.

use crate::check;

#[test]
fn intermediate_page_refinement() {
    check("lenient_intermediate_page_refinement", "truchet");
}

#[test]
fn unknown_length_intermediate() {
    check("lenient_unknown_length_intermediate", "truchet");
}

#[test]
fn missing_reference() {
    check("lenient_missing_reference", "honeycomb");
}

#[test]
fn refinement_of_immediate_region() {
    check("lenient_refinement_of_immediate_region", "truchet");
}

#[test]
fn necessary_extension() {
    check("lenient_necessary_extension", "honeycomb");
}

#[test]
fn reserved_region_flags() {
    check("lenient_reserved_region_flags", "truchet");
}

#[test]
fn reserved_segment_type() {
    check("lenient_reserved_segment_type", "truchet");
}

#[test]
fn trailing_bytes() {
    check("lenient_trailing_bytes", "honeycomb");
}

#[test]
fn truncated_segment() {
    check("lenient_truncated_segment", "truchet");
}

#[test]
fn missing_final_oob() {
    check("lenient_missing_final_oob", "honeycomb");
}

#[test]
fn missing_final_oob_huffman() {
    check("lenient_missing_final_oob_huffman", "honeycomb");
}

#[test]
fn table_past_hthigh() {
    check("lenient_table_past_hthigh", "honeycomb");
}
