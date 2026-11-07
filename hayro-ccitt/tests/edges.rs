use crate::{G3_1D, G3_2D, check};

#[test]
fn g4() {
    check("edges_g4", "edges", |_| {});
}

#[test]
fn g3_1d() {
    check("edges_g3_1d", "edges", |s| s.encoding = G3_1D);
}

#[test]
fn g3_2d() {
    check("edges_g3_2d", "edges", |s| s.encoding = G3_2D);
}
