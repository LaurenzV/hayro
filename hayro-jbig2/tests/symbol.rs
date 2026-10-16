//! Symbol dictionaries (6.5): refinement/aggregate coding, context reuse and segment numbering.

use crate::check;

#[test]
fn basic() {
    check("symbol", "honeycomb");
}

#[test]
fn empty() {
    check("symbol_empty", "honeycomb");
}

#[test]
fn many_dictionaries() {
    check("symbol_many_dictionaries", "honeycomb");
}

#[test]
fn large_segment_numbers() {
    check("symbol_large_segment_numbers", "honeycomb");
}

#[test]
fn template2() {
    check("symbol_template2", "honeycomb");
}

#[test]
fn custom_at() {
    check("symbol_custom_at", "honeycomb");
}

#[test]
fn refine_one() {
    check("symbol_refine_one", "honeycomb");
}

#[test]
fn refine_offsets() {
    check("symbol_refine_offsets", "honeycomb");
}

#[test]
fn huffman_refine_offsets() {
    check("symbol_huffman_refine_offsets", "honeycomb");
}

#[test]
fn refine_one_custom_at() {
    check("symbol_refine_one_custom_at", "honeycomb");
}

#[test]
fn refine_one_template1() {
    check("symbol_refine_one_template1", "honeycomb");
}

#[test]
fn aggregate() {
    check("symbol_aggregate", "honeycomb");
}

#[test]
fn aggregate_then_refine() {
    check("symbol_aggregate_then_refine", "honeycomb");
}

#[test]
fn huffman_aggregate_then_refine() {
    check("symbol_huffman_aggregate_then_refine", "honeycomb");
}

#[test]
fn refine_text_refine() {
    check("symbol_refine_text_refine", "honeycomb");
}

#[test]
fn huffman_refine_one() {
    check("symbol_huffman_refine_one", "honeycomb");
}

#[test]
fn huffman_aggregate() {
    check("symbol_huffman_aggregate", "honeycomb");
}

#[test]
fn huffman_refine_text_refine() {
    check("symbol_huffman_refine_text_refine", "honeycomb");
}

#[test]
fn refine_text_refine_export() {
    check("symbol_refine_text_refine_export", "honeycomb");
}

#[test]
fn huffman_refine_text_refine_export() {
    check("symbol_huffman_refine_text_refine_export", "honeycomb");
}

#[test]
fn global() {
    check("symbol_global", "honeycomb");
}

#[test]
#[ignore = "hayro limits symbol dictionaries to 65535 symbols"]
fn many_symbols() {
    check("symbol_many_symbols", "honeycomb");
}

#[test]
fn context_reuse() {
    check("symbol_context_reuse", "honeycomb");
}

#[test]
fn context_reuse_refine() {
    check("symbol_context_reuse_refine", "honeycomb");
}

#[test]
fn huffman_refine_after_arithmetic() {
    check("symbol_huffman_refine_after_arithmetic", "honeycomb");
}

#[test]
fn huffman_context_reuse_refine() {
    check("symbol_huffman_context_reuse_refine", "honeycomb");
}

#[test]
fn huffman_context_reuse_refine_table() {
    check("symbol_huffman_context_reuse_refine_table", "honeycomb");
}
