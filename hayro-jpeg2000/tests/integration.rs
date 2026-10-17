//! The 8-bit output of the `image` crate integration.

use crate::*;

#[test]
fn gray_alpha() {
    check_output("jp2_gray_alpha", reference("main_ra"));
}

#[test]
fn scaled_12bit() {
    check_output("codestream_precision_12", reference("main_r_12bit"));
}

#[test]
fn scaled_mixed() {
    check_output("jp2_bpcc", reference("main_rgba_5_5_5_1bit"));
}

#[test]
fn cmyk() {
    check_output(
        "integration_cmyk",
        reference("integration_cmyk").tolerance(2),
    );
}

#[test]
fn cmyk_alpha() {
    check_output(
        "integration_cmyk_alpha",
        reference("integration_cmyk_alpha").tolerance(2),
    );
}

#[test]
fn icc_rgb() {
    check_output("integration_icc_rgb", reference("integration_icc_rgb"));
}

#[test]
fn icc_method_3_ignored() {
    check_output(
        "integration_icc_method_3_ignored",
        reference("integration_icc_method_3_ignored"),
    );
}

#[test]
fn cmyk_method_3_ignored() {
    check_output(
        "integration_cmyk_method_3_ignored",
        reference("integration_cmyk").tolerance(2),
    );
}

#[test]
fn colr_romm_rgb() {
    check_output(
        "integration_colr_romm_rgb",
        reference("integration_icc_rgb"),
    );
}

#[test]
fn icc_gray() {
    check_output("integration_icc_gray", reference("integration_icc_gray"));
}

#[test]
fn colr_cielab() {
    check_output(
        "integration_colr_cielab",
        reference("integration_colr_cielab").tolerance(2),
    );
}

#[test]
fn colr_cielab_alpha() {
    check_output(
        "integration_colr_cielab_alpha",
        reference("integration_colr_cielab_alpha").tolerance(2),
    );
}

#[test]
fn colr_cielab_16bit() {
    check_output(
        "integration_colr_cielab_16bit",
        reference("integration_colr_cielab_16bit").tolerance(2),
    );
}

#[test]
fn icc_invalid() {
    check_output("integration_icc_invalid", reference("main_rgb"));
}
