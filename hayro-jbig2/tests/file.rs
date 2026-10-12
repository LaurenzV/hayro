//! File organisation and segment header details (Annex D, 7.2).

use crate::check;

#[test]
fn random_access() {
    check("file_random_access", "truchet");
}

#[test]
fn page_association_32bit() {
    check("file_page_association_32bit", "truchet");
}
