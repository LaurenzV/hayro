use crate::{G3_1D, G3_2D, check};

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
