use crate::{G3_1D, G3_2D, check};

#[test]
fn width_1_g4() {
    check("width_1_g4", "width_1", |_| {});
}

#[test]
fn width_1_g3_1d() {
    check("width_1_g3_1d", "width_1", |s| s.encoding = G3_1D);
}

#[test]
fn width_1_g3_2d() {
    check("width_1_g3_2d", "width_1", |s| s.encoding = G3_2D);
}

#[test]
fn height_1_g4() {
    check("height_1_g4", "height_1", |_| {});
}

#[test]
fn height_1_g3_1d() {
    check("height_1_g3_1d", "height_1", |s| s.encoding = G3_1D);
}

#[test]
fn height_1_g3_2d() {
    check("height_1_g3_2d", "height_1", |s| s.encoding = G3_2D);
}
