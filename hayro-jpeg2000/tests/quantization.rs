//! Irreversible transformation and quantization (Annex E, F.3.8.2).

use crate::*;

#[test]
fn irreversible() {
    check(
        "quantization_irreversible",
        reference("main_r").tolerance(2),
    );
}

#[test]
fn irreversible_12bit() {
    check(
        "quantization_irreversible_12bit",
        reference("main_r_12bit").tolerance(1),
    );
}

#[test]
fn irreversible_16bit() {
    check(
        "quantization_irreversible_16bit",
        reference("stress_16bit").tolerance(1),
    );
}

#[test]
fn scalar_derived() {
    check(
        "quantization_scalar_derived",
        reference("main_r").tolerance(1),
    );
}

#[test]
fn irreversible_tiny_tiles() {
    check(
        "quantization_irreversible_tiny_tiles",
        reference("main_rgb_crop_50_40_12_12").tolerance(2),
    );
}

#[test]
fn lossy_layers() {
    check(
        "quantization_lossy_layers",
        reference("quantization_lossy_layers").tolerance(1),
    );
}

#[test]
fn irreversible_rate() {
    check(
        "quantization_irreversible_rate",
        reference("quantization_irreversible_rate").tolerance(1),
    );
}

#[test]
fn irreversible_tiles_offset() {
    check(
        "quantization_irreversible_tiles_offset",
        reference("quantization_irreversible_tiles_offset"),
    );
}

#[test]
fn irreversible_offset_large() {
    check(
        "quantization_irreversible_offset_large",
        reference("quantization_irreversible_offset_large"),
    );
}

#[test]
fn reversible_lossy_layers() {
    check(
        "quantization_reversible_lossy_layers",
        reference("quantization_reversible_lossy_layers"),
    );
}
