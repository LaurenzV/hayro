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

#[test]
fn page_larger_than_bitmap_limit() {
    // An embedded page information segment (7.4.8) for a page of 100,000 by 100,000
    // pixels, larger than any bitmap the decoder allows. It must be rejected when the page
    // is parsed, before a caller sizes an output buffer by its dimensions.
    let mut data = Vec::new();
    data.extend(0_u32.to_be_bytes()); // segment number
    data.extend([48, 0, 1]); // page information, no referred-to segments, page 1
    data.extend(19_u32.to_be_bytes()); // data length
    data.extend(100_000_u32.to_be_bytes()); // width
    data.extend(100_000_u32.to_be_bytes()); // height
    data.extend([0; 8]); // resolutions
    data.extend([0; 3]); // flags and striping

    let page =
        hayro_jbig2::Page::new_embedded(&data, None, &hayro_jbig2::DecodeSettings::default());
    assert!(page.is_err());
}
