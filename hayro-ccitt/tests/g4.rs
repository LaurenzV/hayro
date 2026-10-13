use crate::{G4, check, check_consumed, check_malformed};

#[test]
fn plain() {
    check("g4", "base", |_| {});
}

#[test]
fn eofb() {
    check("g4_eofb", "base", |s| s.end_of_block = true);
}

#[test]
fn eofb_pad_bits() {
    check_consumed("g4_eofb_pad_bits", "base", Some(1086), |s| {
        s.end_of_block = true;
    });
}

#[test]
fn eofb_trailing_data() {
    check_consumed("g4_eofb_trailing_data", "base", Some(1086), |s| {
        s.end_of_block = true;
    });
}

#[test]
fn byte_align() {
    check("g4_byte_align", "base", |s| s.rows_are_byte_aligned = true);
}

#[test]
fn byte_align_eofb() {
    check("g4_byte_align_eofb", "base", |s| {
        s.rows_are_byte_aligned = true;
        s.end_of_block = true;
    });
}

#[test]
fn eol() {
    check("g4_eol", "base", |s| {
        s.end_of_line = true;
        s.end_of_block = true;
    });
}

#[test]
fn truncated() {
    check_malformed("malformed_g4_truncated", 8, 2, |s| s.encoding = G4);
}
