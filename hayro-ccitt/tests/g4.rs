use crate::{check, check_consumed};

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
