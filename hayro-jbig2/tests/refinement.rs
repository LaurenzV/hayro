//! Generic refinement regions (6.3).

use crate::check;

#[test]
fn trailing_7fff_stripped_many() {
    check("refine_trailing_7fff_stripped_many", "truchet");
}

#[test]
fn template0() {
    check("refine_template0", "truchet");
}

#[test]
fn template1() {
    check("refine_template1", "truchet");
}

#[test]
fn template0_tpgron() {
    check("refine_template0_tpgron", "truchet");
}

#[test]
fn template1_tpgron() {
    check("refine_template1_tpgron", "truchet");
}

#[test]
fn template0_custom_at() {
    check("refine_template0_custom_at", "truchet");
}

#[test]
fn template0_custom_at_tpgron() {
    check("refine_template0_custom_at_tpgron", "truchet");
}

#[test]
fn lossless() {
    check("refine_lossless", "truchet");
}

#[test]
fn refined_region() {
    check("refine_refined_region", "truchet");
}

#[test]
fn page_part_intermediate() {
    check("refine_page_part_intermediate", "truchet");
}

#[test]
fn page() {
    check("refine_page", "truchet");
}

#[test]
fn page_part() {
    check("refine_page_part", "truchet");
}
