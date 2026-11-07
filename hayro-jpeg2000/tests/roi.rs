//! Regions of interest (Annex H).

use crate::*;

#[test]
fn reversible() {
    check("roi_reversible", reference("main_rgb"));
}

#[test]
fn tile_part_override() {
    check("roi_tile_part_override", reference("main_rgb"));
}

#[test]
fn irreversible() {
    check("roi_irreversible", reference("main_rgb").tolerance(3));
}
