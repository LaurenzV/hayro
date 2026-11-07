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
fn page_replace() {
    check("composite_page_replace", "truchet");
}

#[test]
fn page_default_pixel_and() {
    check("composite_page_default_pixel_and", "truchet");
}

#[test]
fn page_default_pixel_xnor() {
    check("composite_page_default_pixel_xnor", "truchet");
}

#[test]
fn page_default_pixel_replace() {
    check("composite_page_default_pixel_replace", "truchet");
}

#[test]
fn region_default_pixel() {
    check("composite_region_default_pixel", "truchet");
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
