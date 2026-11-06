use crate::{G3_1D, G3_2D, G4, check, check_partial};

#[test]
fn corrupt_line_g3_1d() {
    check("g3_1d_corrupt_line", "g3_1d_corrupt_line", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn truncated_line_g3_1d() {
    check("g3_1d_truncated_line", "g3_1d_truncated_line", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn truncated_line_g3_2d() {
    check("g3_2d_truncated_line", "g3_2d_truncated_line", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
    });
}

#[test]
fn eol_flag_without_eols() {
    check("g3_1d", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn run_past_line_end_g3_1d() {
    check("g3_1d_run_past_line_end", "g3_1d_run_past_line_end", |s| {
        s.encoding = G3_1D;
    });
}

#[test]
fn byte_align_flag_without_padding_g3_1d() {
    check("edges_g3_1d", "edges", |s| {
        s.encoding = G3_1D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_flag_without_padding_g3_2d() {
    check("g3_2d", "base", |s| {
        s.encoding = G3_2D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_flag_without_padding_g4() {
    check("g4", "base", |s| {
        s.encoding = G4;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn truncated_line_g4() {
    check_partial("g4_truncated_line", "g4_truncated_line", |s| {
        s.encoding = G4;
    });
}
