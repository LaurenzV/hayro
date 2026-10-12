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
fn refine_one() {
    check("symbol_refine_one", "honeycomb");
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
fn refine_text_refine() {
    check("symbol_refine_text_refine", "honeycomb");
}

#[test]
fn huffman_refine_one() {
    check("symbol_huffman_refine_one", "honeycomb");
}

#[test]
#[ignore = "refinement contexts must not be reset between symbol instances in Huffman text regions (7.4.3.2)"]
fn huffman_aggregate() {
    check("symbol_huffman_aggregate", "honeycomb");
}

#[test]
#[ignore = "refinement contexts must not be reset between symbol instances in Huffman text regions (7.4.3.2)"]
fn huffman_refine_text_refine() {
    check("symbol_huffman_refine_text_refine", "honeycomb");
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
