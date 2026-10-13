//! Text regions (6.4): reference corners, transposition, offsets, refinement and combination operators.

use crate::check;

#[test]
fn bottomleft() {
    check("text_bottomleft", "sphere");
}

#[test]
fn bottomright() {
    check("text_bottomright", "sphere");
}

#[test]
fn topright() {
    check("text_topright", "sphere");
}

#[test]
fn bottomleft_transposed() {
    check("text_bottomleft_transposed", "sphere");
}

#[test]
fn bottomright_transposed() {
    check("text_bottomright_transposed", "sphere");
}

#[test]
fn topright_transposed() {
    check("text_topright_transposed", "sphere");
}

#[test]
fn topleft_transposed() {
    check("text_topleft_transposed", "sphere");
}

#[test]
fn negative_ds_offset() {
    check("text_negative_ds_offset", "honeycomb");
}

#[test]
fn refine() {
    check("text_refine", "honeycomb");
}

#[test]
fn refine_custom_at() {
    check("text_refine_custom_at", "honeycomb");
}

#[test]
fn refine_negative_delta_width() {
    check("text_refine_negative_delta_width", "honeycomb");
}

#[test]
fn then_refine() {
    check("text_then_refine", "honeycomb");
}

#[test]
fn large_integer() {
    check("text_large_integer", "honeycomb");
}

#[test]
fn composition() {
    check("text_composition", "honeycomb");
}
