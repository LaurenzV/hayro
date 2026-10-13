//! Symbol dictionaries and text regions with Huffman coding (Annex B).

use crate::check;

#[test]
#[ignore = "hayro limits symbol dictionaries to 65535 symbols"]
fn many_symbols() {
    check("huffman_many_symbols", "honeycomb");
}

#[test]
fn standard_tables() {
    check("huffman_standard_tables", "honeycomb");
}

#[test]
fn tables_b10_b13() {
    check("huffman_tables_b10_b13", "honeycomb");
}

#[test]
fn tables_b5_b3_b7_b9_b12() {
    check("huffman_tables_b5_b3_b7_b9_b12", "honeycomb");
}

#[test]
fn uncompressed_symbols() {
    check("huffman_uncompressed_symbols", "honeycomb");
}

#[test]
fn custom_tables() {
    check("huffman_custom_tables", "honeycomb");
}

#[test]
fn symbol_id_run_codes() {
    check("huffman_symbol_id_run_codes", "honeycomb");
}

#[test]
fn trailing_unused_symbols() {
    check("huffman_trailing_unused_symbols", "honeycomb");
}

#[test]
fn text_refine() {
    check("huffman_text_refine", "honeycomb");
}

#[test]
fn text_refine_b15() {
    check("huffman_text_refine_b15", "honeycomb");
}

#[test]
fn text_refine_custom() {
    check("huffman_text_refine_custom", "honeycomb");
}

#[test]
fn text_refine_custom_offsets() {
    check("huffman_text_refine_custom_offsets", "honeycomb");
}

#[test]
fn text_refine_custom_sizes() {
    check("huffman_text_refine_custom_sizes", "honeycomb");
}

#[test]
fn text_refine_custom_sizes_offsets() {
    check("huffman_text_refine_custom_sizes_offsets", "honeycomb");
}

#[test]
fn text_refine_custom_data_size() {
    check("huffman_text_refine_custom_data_size", "honeycomb");
}

#[test]
fn text_refine_global_tables() {
    check("huffman_text_refine_global_tables", "honeycomb");
}
