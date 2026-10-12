//! Combining regions on the page (clause 8).

use crate::check;

#[test]
fn or_xor_replace() {
    check("composite_or_xor_replace", "truchet");
}

#[test]
fn and_xnor() {
    check("composite_and_xnor", "truchet");
}

#[test]
fn or_xor_replace_refine() {
    check("composite_or_xor_replace_refine", "truchet");
}

#[test]
fn and_xnor_refine() {
    check("composite_and_xnor_refine", "truchet");
}

#[test]
fn or_xor_replace_text() {
    check("composite_or_xor_replace_text", "truchet");
}

#[test]
fn and_xnor_text() {
    check("composite_and_xnor_text", "truchet");
}

#[test]
fn or_xor_replace_halftone() {
    check("composite_or_xor_replace_halftone", "truchet");
}

#[test]
fn and_xnor_halftone() {
    check("composite_and_xnor_halftone", "truchet");
}
