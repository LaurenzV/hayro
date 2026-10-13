//! Malformed streams, which must be rejected or decoded without panicking.

use crate::check_malformed;

#[test]
fn reused_contexts_of_another_template() {
    check_malformed("malformed_reused_contexts_of_another_template");
}
