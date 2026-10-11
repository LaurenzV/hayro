use crate::{G3_2D, check};

#[test]
fn pass_g4() {
    check("mode_pass_g4", "mode_pass", |_| {});
}

#[test]
fn pass_g3_2d() {
    check("mode_pass_g3_2d", "mode_pass", |s| s.encoding = G3_2D);
}

#[test]
fn vertical_g4() {
    check("mode_vertical_g4", "mode_vertical", |_| {});
}

#[test]
fn vertical_g3_2d() {
    check("mode_vertical_g3_2d", "mode_vertical", |s| {
        s.encoding = G3_2D;
    });
}

#[test]
fn horizontal_g4() {
    check("mode_horizontal_g4", "mode_horizontal", |_| {});
}

#[test]
fn horizontal_g3_2d() {
    check("mode_horizontal_g3_2d", "mode_horizontal", |s| {
        s.encoding = G3_2D;
    });
}
