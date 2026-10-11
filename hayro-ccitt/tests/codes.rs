use crate::{G3_1D, G3_2D, check};

#[test]
fn g3_1d() {
    check("codes_g3_1d", "wedge", |s| s.encoding = G3_1D);
}

#[test]
fn g3_2d() {
    check("codes_g3_2d", "wedge", |s| s.encoding = G3_2D);
}

#[test]
fn g4() {
    check("codes_g4", "wedge", |_| {});
}
