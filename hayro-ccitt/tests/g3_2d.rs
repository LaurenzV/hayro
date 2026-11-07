use crate::{G3_2D, check};

#[test]
fn plain() {
    check("g3_2d", "base", |s| s.encoding = G3_2D);
}

#[test]
fn eol() {
    check("g3_2d_eol", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
    });
}

#[test]
fn eol_fill() {
    check("g3_2d_eol_fill", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
    });
}

#[test]
fn rtc() {
    check("g3_2d_rtc", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
        s.end_of_block = true;
    });
}

#[test]
fn byte_align() {
    check("g3_2d_byte_align", "base", |s| {
        s.encoding = G3_2D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_eol() {
    check("g3_2d_byte_align_eol", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_eol_fill() {
    check("g3_2d_byte_align_eol_fill", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn all_1d() {
    check("g3_2d_all_1d", "base", |s| s.encoding = G3_2D);
}

#[test]
fn irregular_tags() {
    check("g3_2d_irregular_tags", "base", |s| s.encoding = G3_2D);
}
