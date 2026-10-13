//! Invalid streams that are decoded leniently, like most other decoders do.

use crate::check;

#[test]
fn intermediate_page_refinement() {
    check("lenient_intermediate_page_refinement", "truchet");
}
