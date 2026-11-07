//! Code-block coding styles (Annex D.6, Table A.19).

use crate::*;

#[test]
fn bypass() {
    check("code_blocks_bypass", reference("main_r"));
}

#[test]
fn reset() {
    check("code_blocks_reset", reference("main_r"));
}

#[test]
fn termall() {
    check("code_blocks_termall", reference("main_r"));
}

#[test]
fn vcausal() {
    check("code_blocks_vcausal", reference("main_r"));
}

#[test]
fn pterm() {
    check("code_blocks_pterm", reference("main_r"));
}

#[test]
fn segsym() {
    check("code_blocks_segsym", reference("main_r"));
}

#[test]
fn bypass_termall() {
    check("code_blocks_bypass_termall", reference("main_r"));
}

#[test]
fn bypass_pterm() {
    check("code_blocks_bypass_pterm", reference("main_r"));
}

#[test]
fn termall_pterm() {
    check("code_blocks_termall_pterm", reference("main_r"));
}

#[test]
fn bypass_termall_pterm() {
    check("code_blocks_bypass_termall_pterm", reference("main_r"));
}

#[test]
fn reset_termall() {
    check("code_blocks_reset_termall", reference("main_r"));
}

#[test]
fn termall_segsym() {
    check("code_blocks_termall_segsym", reference("main_r"));
}

#[test]
fn reset_termall_segsym() {
    check("code_blocks_reset_termall_segsym", reference("main_r"));
}

#[test]
fn all_but_termall() {
    check("code_blocks_all_but_termall", reference("main_r"));
}

#[test]
fn all_but_pterm() {
    check("code_blocks_all_but_pterm", reference("main_r"));
}

#[test]
fn all_styles() {
    check("code_blocks_all_styles", reference("main_r"));
}

#[test]
fn termall_layers() {
    check("code_blocks_termall_layers", reference("main_r"));
}

#[test]
fn bypass_layers() {
    check("code_blocks_bypass_layers", reference("main_r"));
}

#[test]
fn bypass_segment_spans_layers() {
    check(
        "code_blocks_bypass_segment_spans_layers",
        reference("main_r"),
    );
}

#[test]
fn missing_bitplanes_exceed() {
    check(
        "code_blocks_missing_bitplanes_exceed",
        reference("code_blocks_missing_bitplanes_exceed"),
    );
}

#[test]
fn bypass_data_exhausted() {
    check("code_blocks_bypass_data_exhausted", reference("main_rgb"));
}

#[test]
fn no_coding_passes() {
    check(
        "code_blocks_no_coding_passes",
        reference("code_blocks_no_coding_passes"),
    );
}
