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

/// A 97-byte codestream claiming a 100000x100000 image.
#[test]
fn too_large() {
    check_error(
        "errors_too_large",
        DecodeSettings::default(),
        "image is too large",
    );
}

#[test]
fn too_large_accepted_with_larger_budget() {
    let data = read_stream("errors_too_large");
    let settings = DecodeSettings {
        max_decoded_samples: u64::MAX,
        ..Default::default()
    };
    let image = Image::new(&data, &settings).unwrap();
    assert_eq!((image.width(), image.height()), (100_000, 100_000));
}

/// `codestream_gray` is 128x96 with one component and one tile: 12288 decoded
/// samples plus at most 129x97 coefficients.
#[test]
fn budget_boundary() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            max_decoded_samples: 24_801,
            ..Default::default()
        },
        reference("main_r"),
    );
    check_error(
        "codestream_gray",
        DecodeSettings {
            max_decoded_samples: 24_800,
            ..Default::default()
        },
        "image is too large",
    );
}
