//! Palettes (I.5.3.4, I.5.3.5).

use crate::*;

#[test]
fn basic() {
    check("palette_basic", reference("palette_basic"));
}

#[test]
fn tiled() {
    check("palette_tiled", reference("palette_basic"));
}

#[test]
fn small() {
    check("palette_small", reference("palette_small"));
}

#[test]
fn four_bit_entries() {
    check(
        "palette_four_bit_entries",
        reference("palette_four_bit_entries"),
    );
}

#[test]
fn out_of_range_indices() {
    check(
        "palette_out_of_range_indices",
        reference("palette_out_of_range_indices"),
    );
}

#[test]
fn cdef() {
    check("palette_cdef", reference("palette_basic"));
}

#[test]
fn cmap_outside_jp2h() {
    check("palette_cmap_outside_jp2h", reference("palette_basic"));
}

#[test]
fn unresolved() {
    check_with(
        "palette_basic",
        DecodeSettings {
            resolve_palette_indices: false,
            ..Default::default()
        },
        reference("palette_unresolved"),
    );
}
