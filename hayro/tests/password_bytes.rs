//! Render encrypted files opened with already encoded passwords.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderSettings, render_pdf};
use std::fs::read;

#[test]
fn encoded_legacy_passwords_render_the_same_pages_as_the_plain_source() {
    let data = read("../tests/pdfs/other/password_bytes_plain.pdf").unwrap();
    let plain = Pdf::new(data).unwrap();
    let settings = InterpreterSettings::default();
    let rendered = |pdf: &Pdf| {
        render_pdf(
            pdf,
            0.25,
            settings.clone(),
            &RenderSettings::default(),
            None,
        )
        .unwrap()
    };
    let expected = rendered(&plain);
    assert_eq!(expected.len(), 2);
    assert_eq!(expected[0].width(), expected[1].height());
    assert_eq!(expected[0].height(), expected[1].width());

    for revision in [2, 3, 4] {
        let file = format!("../tests/pdfs/other/password_bytes_r{revision}.pdf");
        let pdf = Pdf::new_with_password_bytes(read(file).unwrap(), &[0xe9]).unwrap();
        let actual = rendered(&pdf);
        assert_eq!(actual.len(), expected.len());
        for (page, control) in actual.iter().zip(&expected) {
            assert_eq!(
                (page.width(), page.height()),
                (control.width(), control.height())
            );
            assert!(control.data_as_u8_slice().iter().any(|&v| v != 255));
            assert_eq!(page.data_as_u8_slice(), control.data_as_u8_slice());
        }
    }
}
