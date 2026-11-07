use crate::{G3_1D, G3_2D, check, check_consumed};

#[test]
fn rows_fewer_than_data() {
    check_consumed("g4", "base", Some(127), |s| s.rows = 48);
}

#[test]
fn eofb_before_rows() {
    check("g4_eofb", "base", |s| {
        s.end_of_block = true;
        s.rows = 200;
    });
}

#[test]
fn rows_unknown_rtc() {
    check("g3_1d_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
        s.rows = 0;
    });
}

#[test]
fn end_of_block_without_eofb() {
    check("g4", "base", |s| s.end_of_block = true);
}

#[test]
fn end_of_block_without_rtc() {
    check("g3_1d_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
    });
}

#[test]
fn rows_unknown_rtc_2d() {
    check("g3_2d_rtc", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
        s.end_of_block = true;
        s.rows = 0;
    });
}
