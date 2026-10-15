//! Multiple component transformations (Annex G).

use crate::*;

#[test]
fn rct() {
    check("mct_rct", reference("main_rgb"));
}

#[test]
fn rct_four_components() {
    check("mct_rct_four_components", reference("main_rgba"));
}

#[test]
fn rct_five_components() {
    check_components("mct_rct_five_components", reference("main_rgbar"));
}

#[test]
fn signed_components() {
    check("mct_signed_components", reference("main_rgb"));
}

#[test]
fn one_component_ignored() {
    check("mct_one_component_ignored", reference("main_r"));
}

#[test]
fn raw_ycbcr_guess() {
    check("mct_raw_ycbcr_guess", reference("mct_raw_ycbcr_guess"));
}

#[test]
fn ict() {
    check("mct_ict", reference("main_rgb").tolerance(3));
}
