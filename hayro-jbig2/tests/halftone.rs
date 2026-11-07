//! Pattern dictionaries and halftone regions (6.6, 6.7).

use crate::{check, check_crop};

#[test]
fn basic() {
    check("halftone", "maze");
}

#[test]
fn replace_word_aligned() {
    check("halftone_replace_word_aligned", "maze");
}

#[test]
fn global() {
    check("halftone_global", "maze");
}

#[test]
fn template1() {
    check("halftone_template1", "maze");
}

#[test]
fn template2() {
    check("halftone_template2", "maze");
}

#[test]
fn template3() {
    check("halftone_template3", "maze");
}

#[test]
fn halftone_10_bitplanes() {
    check("halftone_10_bitplanes", "maze");
}

#[test]
fn halftone_10_bitplanes_mmr() {
    check("halftone_10_bitplanes_mmr", "maze");
}

#[test]
fn mmr_bitplane_boundary() {
    check_crop("halftone_mmr_bitplane_boundary", "maze", (63, 0, 27, 9));
}

#[test]
fn skip_unused() {
    check("halftone_skip_unused", "maze");
}

#[test]
fn then_refine() {
    check("halftone_then_refine", "maze");
}

#[test]
fn then_refine_default_pixel() {
    check("halftone_then_refine_default_pixel", "maze");
}

#[test]
fn rotated_grid() {
    check("halftone_rotated_grid", "sphere");
}

#[test]
fn rotated_grid_skip() {
    check("halftone_rotated_grid_skip", "sphere");
}

#[test]
fn rotated_grid_skip_template1() {
    check("halftone_rotated_grid_skip_template1", "sphere");
}

#[test]
fn rotated_grid_skip_template2() {
    check("halftone_rotated_grid_skip_template2", "sphere");
}

#[test]
fn rotated_grid_skip_template3() {
    check("halftone_rotated_grid_skip_template3", "sphere");
}

#[test]
fn combination_operators() {
    check("halftone_combination_operators", "truchet");
}

#[test]
fn tiles_xor() {
    check("halftone_tiles_xor", "truchet");
}

#[test]
fn tiles_and() {
    check("halftone_tiles_and", "truchet");
}

#[test]
fn tiles_xnor() {
    check("halftone_tiles_xnor", "truchet");
}

#[test]
fn tiles_replace() {
    check("halftone_tiles_replace", "truchet");
}

#[test]
fn large_patterns() {
    check("halftone_large_patterns", "truchet");
}

#[test]
fn tall_patterns() {
    check("halftone_tall_patterns", "truchet");
}
