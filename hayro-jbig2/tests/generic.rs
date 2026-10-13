//! Generic regions (6.2): templates, adaptive template pixels, TPGDON and MMR.

use crate::{check, check_crop};

#[test]
fn template0() {
    check("generic_template0", "truchet");
}

#[test]
fn template1() {
    check("generic_template1", "truchet");
}

#[test]
fn template1_custom_at() {
    check("generic_template1_custom_at", "truchet");
}

#[test]
fn template1_custom_at_tpgdon() {
    check("generic_template1_custom_at_tpgdon", "maze");
}

#[test]
fn template2() {
    check("generic_template2", "truchet");
}

#[test]
fn template2_custom_at() {
    check("generic_template2_custom_at", "truchet");
}

#[test]
fn template2_custom_at_tpgdon() {
    check("generic_template2_custom_at_tpgdon", "maze");
}

#[test]
fn template3() {
    check("generic_template3", "truchet");
}

#[test]
fn template3_custom_at() {
    check("generic_template3_custom_at", "truchet");
}

#[test]
fn template3_custom_at_tpgdon() {
    check("generic_template3_custom_at_tpgdon", "maze");
}

#[test]
fn template0_custom_at() {
    check("generic_template0_custom_at", "truchet");
}

#[test]
fn template0_custom_at_tpgdon() {
    check("generic_template0_custom_at_tpgdon", "maze");
}

#[test]
fn template0_tpgdon() {
    check("generic_template0_tpgdon", "maze");
}

#[test]
fn template1_tpgdon() {
    check("generic_template1_tpgdon", "maze");
}

#[test]
fn template2_tpgdon() {
    check("generic_template2_tpgdon", "maze");
}

#[test]
fn template3_tpgdon() {
    check("generic_template3_tpgdon", "maze");
}

#[test]
fn mmr() {
    check("generic_mmr", "truchet");
}

#[test]
fn unknown_length() {
    check("generic_unknown_length", "truchet");
}

#[test]
fn unknown_length_custom_at() {
    check("generic_unknown_length_custom_at", "truchet");
}

#[test]
fn trailing_7fff_stripped() {
    check("generic_trailing_7fff_stripped", "honeycomb");
}

#[test]
fn trailing_7fff_stripped_many() {
    check("generic_trailing_7fff_stripped_many", "truchet");
}

#[test]
fn row_ending_in_full_chunks() {
    check_crop(
        "generic_row_ending_in_full_chunks",
        "truchet",
        (0, 30, 32, 23),
    );
}
