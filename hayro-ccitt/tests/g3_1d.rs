use crate::{G3_1D, check};

#[test]
fn plain() {
    check("g3_1d", "base", |s| s.encoding = G3_1D);
}

#[test]
fn eol() {
    check("g3_1d_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn eol_fill() {
    check("g3_1d_eol_fill", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn long_fill() {
    check("g3_1d_long_fill", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn rtc() {
    check("g3_1d_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
    });
}

#[test]
fn byte_align() {
    check("g3_1d_byte_align", "base", |s| {
        s.encoding = G3_1D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_eol() {
    check("g3_1d_byte_align_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn byte_align_rtc() {
    check("g3_1d_byte_align_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
        s.rows_are_byte_aligned = true;
    });
}
