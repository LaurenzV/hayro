//! Page striping (7.4.8.6, 7.4.10).

use crate::check;

#[test]
fn striped() {
    check("page_striped", "truchet");
}

#[test]
fn striped_unknown_height() {
    check("page_striped_unknown_height", "truchet");
}

#[test]
fn striped_last_stripe_implicit() {
    check("page_striped_last_stripe_implicit", "truchet");
}

#[test]
fn striped_single_stripe() {
    check("page_striped_single_stripe", "truchet");
}

#[test]
fn striped_no_end_of_stripe() {
    check("page_striped_no_end_of_stripe", "truchet");
}
