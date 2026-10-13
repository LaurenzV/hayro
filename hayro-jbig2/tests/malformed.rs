//! Malformed streams, which must be rejected or decoded without panicking.

use crate::check_malformed;

#[test]
fn reused_contexts_of_another_template() {
    check_malformed("malformed_reused_contexts_of_another_template");
}

#[test]
fn mmr_region_with_height_0() {
    check_malformed("malformed_mmr_region_with_height_0");
}

#[test]
fn huffman_value_out_of_range() {
    check_malformed("malformed_huffman_value_out_of_range");
}

#[test]
fn text_placement_out_of_range() {
    check_malformed("malformed_text_placement_out_of_range");
}
