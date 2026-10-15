//! Decoding at a reduced resolution (F.1, B.5).

use crate::*;

#[test]
fn reduced_resolution_1() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((64, 48)),
            ..Default::default()
        },
        reference("wavelet_reduced_resolution_1"),
    );
}

#[test]
fn reduced_resolution_2() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((32, 24)),
            ..Default::default()
        },
        reference("wavelet_reduced_resolution_2"),
    );
}

#[test]
fn reduced_resolution_3() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((16, 12)),
            ..Default::default()
        },
        reference("wavelet_reduced_resolution_3"),
    );
}

#[test]
fn reduced_resolution_4() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((8, 6)),
            ..Default::default()
        },
        reference("wavelet_reduced_resolution_4"),
    );
}

#[test]
fn reduced_lowest() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((4, 3)),
            ..Default::default()
        },
        reference("wavelet_reduced_lowest"),
    );
}

#[test]
fn reduced_subsampled() {
    check_with(
        "codestream_subsampling_with_offset",
        DecodeSettings {
            target_resolution: Some((12, 16)),
            ..Default::default()
        },
        reference("wavelet_reduced_subsampled"),
    );
}

#[test]
fn reduced_offset() {
    check_with(
        "codestream_offset_large",
        DecodeSettings {
            target_resolution: Some((32, 24)),
            ..Default::default()
        },
        reference("wavelet_reduced_offset"),
    );
}

#[test]
#[ignore = "hayro leaves grid positions before the first sample of a subsampled component empty"]
fn reduced_precinct_limited() {
    check_with(
        "wavelet_reduced_precinct_limited",
        DecodeSettings {
            target_resolution: Some((4, 6)),
            ..Default::default()
        },
        reference("wavelet_reduced_precinct_limited"),
    );
}

#[test]
fn reduced_target_larger_than_image() {
    check_with(
        "codestream_gray",
        DecodeSettings {
            target_resolution: Some((1000, 1000)),
            ..Default::default()
        },
        reference("main_r"),
    );
}

#[test]
fn reduced_palette() {
    check_with(
        "palette_small",
        DecodeSettings {
            target_resolution: Some((1, 1)),
            ..Default::default()
        },
        reference("palette_small"),
    );
}

#[test]
fn reduced_per_component_levels() {
    check_with(
        "codestream_levels_per_component",
        DecodeSettings {
            target_resolution: Some((64, 48)),
            ..Default::default()
        },
        reference("wavelet_reduced_per_component_levels"),
    );
}
