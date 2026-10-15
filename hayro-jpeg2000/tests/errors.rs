//! Streams that must be rejected.

use crate::*;

#[test]
fn too_few_exponents() {
    check_error(
        "errors_too_few_exponents",
        DecodeSettings::default(),
        "not enough exponents provided in header",
    );
}

#[test]
fn reduced_below_tile_levels() {
    check_error(
        "codestream_tile_cod_fewer_levels",
        DecodeSettings {
            target_resolution: Some((32, 24)),
            ..Default::default()
        },
        "invalid component metadata",
    );
}

#[test]
fn jp2_invalid_codestream() {
    check_error(
        "errors_jp2_invalid_codestream",
        DecodeSettings::default(),
        "failed to parse COD marker",
    );
}
