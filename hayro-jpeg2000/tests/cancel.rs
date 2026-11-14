use hayro_jpeg2000::CancelHandle;

use crate::*;

#[test]
fn cancelled() {
    check_error(
        "code_blocks_all_styles",
        DecodeSettings {
            cancel: Some(CancelHandle::new(|| true)),
            ..Default::default()
        },
        "decoding was cancelled",
    );
}
