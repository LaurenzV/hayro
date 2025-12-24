use hayro::InterpreterSettings;
use hayro::Pdf;
use hayro::render_pdf;
use hayro_jpeg2000::{DecodeSettings, Image};
use hayro_syntax::metadata::Metadata;
use hayro_syntax::object::DateTime;
use std::sync::Arc;

fn load_pdf(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    if let Ok(pdf) = pdf {
        let _pixmaps = render_pdf(&pdf, 1.0, InterpreterSettings::default(), None);
    }
}

fn load_jpeg2000(file: &[u8]) {
    use image::ImageDecoder;

    let settings = DecodeSettings::default();
    if let Ok(image) = Image::new(file, &settings) {
        let mut buf = vec![0_u8; image.total_bytes() as usize];
        let _ = image.read_image(&mut buf);
    }
}

#[test]
fn issue50() {
    let file = include_bytes!("../pdfs/load/issue50.pdf");
    load_pdf(file);
}

#[test]
fn issue52() {
    let file = include_bytes!("../pdfs/load/issue52.pdf");
    load_pdf(file);
}

#[test]
fn issue54() {
    let file = include_bytes!("../pdfs/load/issue54.pdf");
    load_pdf(file);
}

#[test]
fn issue55() {
    let file = include_bytes!("../pdfs/load/issue55.pdf");
    load_pdf(file);
}

#[test]
fn issue56() {
    let file = include_bytes!("../pdfs/load/issue56.pdf");
    load_pdf(file);
}

#[test]
fn issue61() {
    let file = include_bytes!("../pdfs/load/issue61.pdf");
    load_pdf(file);
}

#[test]
fn issue62() {
    let file = include_bytes!("../pdfs/load/issue62.pdf");
    load_pdf(file);
}

#[test]
fn issue63() {
    let file = include_bytes!("../pdfs/load/issue63.pdf");
    load_pdf(file);
}

#[test]
fn issue67() {
    let file = include_bytes!("../pdfs/load/issue67.pdf");
    load_pdf(file);
}

#[test]
fn issue68() {
    let file = include_bytes!("../pdfs/load/issue68.pdf");
    load_pdf(file);
}

#[test]
fn issue83() {
    let file = include_bytes!("../pdfs/load/issue83.pdf");
    load_pdf(file);
}

#[test]
fn issue152() {
    let file = include_bytes!("../pdfs/load/issue152.pdf");
    load_pdf(file);
}

#[test]
fn issue153() {
    let file = include_bytes!("../pdfs/load/issue153.pdf");
    load_pdf(file);
}

#[test]
fn issue154() {
    let file = include_bytes!("../pdfs/load/issue154.pdf");
    load_pdf(file);
}

#[test]
fn issue156() {
    let file = include_bytes!("../pdfs/load/issue156.pdf");
    load_pdf(file);
}

#[test]
fn issue157() {
    let file = include_bytes!("../pdfs/load/issue157.pdf");
    load_pdf(file);
}

#[test]
fn issue178() {
    let file = include_bytes!("../pdfs/load/issue178.pdf");
    load_pdf(file);
}

#[test]
fn issue180() {
    let file = include_bytes!("../pdfs/load/issue180.pdf");
    load_pdf(file);
}

#[test]
fn issue182() {
    let file = include_bytes!("../pdfs/load/issue182.pdf");
    load_pdf(file);
}

#[test]
fn issue203() {
    let file = include_bytes!("../pdfs/load/issue203.pdf");
    load_pdf(file);
}

#[test]
fn issue204() {
    let file = include_bytes!("../pdfs/load/issue204.pdf");
    load_pdf(file);
}

#[test]
fn issue205() {
    let file = include_bytes!("../pdfs/load/issue205.pdf");
    load_pdf(file);
}

#[test]
fn issue206() {
    let file = include_bytes!("../pdfs/load/issue206.pdf");
    load_pdf(file);
}

#[test]
fn issue207() {
    let file = include_bytes!("../pdfs/load/issue207.pdf");
    load_pdf(file);
}

#[test]
fn issue208() {
    let file = include_bytes!("../pdfs/load/issue208.pdf");
    load_pdf(file);
}

#[test]
fn issue222() {
    let file = include_bytes!("../pdfs/load/issue222.pdf");
    load_pdf(file);
}

#[test]
fn issue223() {
    let file = include_bytes!("../pdfs/load/issue223.pdf");
    load_pdf(file);
}

#[test]
fn issue224() {
    let file = include_bytes!("../pdfs/load/issue224.pdf");
    load_pdf(file);
}

#[test]
fn issue234() {
    let file = include_bytes!("../pdfs/load/issue234.pdf");
    load_pdf(file);
}

#[test]
fn issue235() {
    let file = include_bytes!("../pdfs/load/issue235.pdf");
    load_pdf(file);
}

#[test]
fn issue236() {
    let file = include_bytes!("../pdfs/load/issue236.pdf");
    load_pdf(file);
}

#[test]
fn issue256() {
    let file = include_bytes!("../pdfs/load/issue256.pdf");
    load_pdf(file);
}

#[test]
fn issue273_180b() {
    let file = include_bytes!("../pdfs/load/issue273_180b.pdf");
    load_pdf(file);
}

#[test]
fn issue323() {
    let file = include_bytes!("../pdfs/load/issue323.pdf");
    load_pdf(file);
}

#[test]
fn issue324() {
    let file = include_bytes!("../pdfs/load/issue324.pdf");
    load_pdf(file);
}

#[test]
fn issue325() {
    let file = include_bytes!("../pdfs/load/issue325.pdf");
    load_pdf(file);
}

#[test]
fn issue351() {
    let file = include_bytes!("../pdfs/load/issue351.pdf");
    load_pdf(file);
}

#[test]
fn issue352() {
    let file = include_bytes!("../pdfs/load/issue352.pdf");
    load_pdf(file);
}

#[test]
fn issue355() {
    let file = include_bytes!("../pdfs/load/issue355.pdf");
    load_pdf(file);
}

#[test]
fn issue356() {
    let file = include_bytes!("../pdfs/load/issue356.pdf");
    load_pdf(file);
}

#[test]
fn issue357() {
    let file = include_bytes!("../pdfs/load/issue357.pdf");
    load_pdf(file);
}

#[test]
fn issue372() {
    let file = include_bytes!("../pdfs/load/issue372.pdf");
    load_pdf(file);
}

#[test]
fn issue389() {
    let file = include_bytes!("../pdfs/load/issue389.pdf");
    load_pdf(file);
}

#[test]
fn issue390() {
    let file = include_bytes!("../pdfs/load/issue390.pdf");
    load_pdf(file);
}

#[test]
fn issue391() {
    let file = include_bytes!("../pdfs/load/issue391.pdf");
    load_pdf(file);
}

#[test]
fn issue409() {
    let file = include_bytes!("../pdfs/load/issue409.pdf");
    load_pdf(file);
}

#[test]
fn issue472() {
    let file = include_bytes!("../pdfs/load/issue472.pdf");
    load_pdf(file);
}

#[test]
fn issue506() {
    let file = include_bytes!("../pdfs/load/issue506.pdf");
    load_pdf(file);
}

#[test]
fn issue507() {
    let file = include_bytes!("../pdfs/load/issue507.pdf");
    load_pdf(file);
}

#[test]
fn issue513() {
    let file = include_bytes!("../pdfs/load/issue513.pdf");
    load_pdf(file);
}

#[test]
fn issue514() {
    let file = include_bytes!("../pdfs/load/issue514.pdf");
    load_pdf(file);
}

#[test]
fn issue515() {
    let file = include_bytes!("../pdfs/load/issue515.pdf");
    load_pdf(file);
}

#[test]
fn issue520() {
    let file = include_bytes!("../pdfs/load/issue520.pdf");
    load_pdf(file);
}

#[test]
fn issue538() {
    let file = include_bytes!("../pdfs/load/issue538.pdf");
    load_pdf(file);
}

#[test]
fn issue563() {
    let file = include_bytes!("../pdfs/load/issue563.pdf");
    load_pdf(file);
}

#[test]
fn issue564() {
    let file = include_bytes!("../pdfs/load/issue564.pdf");
    load_pdf(file);
}

#[test]
fn issue577() {
    let file = include_bytes!("../pdfs/load/issue577.pdf");
    load_pdf(file);
}

#[test]
fn issue578() {
    let file = include_bytes!("../pdfs/load/issue578.pdf");
    load_pdf(file);
}

#[test]
fn issue579() {
    let file = include_bytes!("../pdfs/load/issue579.pdf");
    load_pdf(file);
}

#[test]
fn issue585() {
    let file = include_bytes!("../pdfs/load/issue585.pdf");
    load_pdf(file);
}

#[test]
fn page_tree_cycle() {
    let file = include_bytes!("../pdfs/load/page_tree_cycle.pdf");
    load_pdf(file);
}

#[test]
fn issue645() {
    let file = include_bytes!("../pdfs/load/issue645.pdf");
    load_pdf(file);
}

#[test]
fn issue675() {
    let file = include_bytes!("../pdfs/load/issue675.pdf");
    load_pdf(file);
}

#[test]
fn issue677() {
    let file = include_bytes!("../pdfs/load/issue677.pdf");
    load_pdf(file);
}

#[test]
fn issue678() {
    let file = include_bytes!("../pdfs/load/issue678.pdf");
    load_pdf(file);
}

#[test]
fn issue679() {
    let file = include_bytes!("../pdfs/load/issue679.pdf");
    load_pdf(file);
}

#[test]
fn issue680() {
    let file = include_bytes!("../pdfs/load/issue680.pdf");
    load_pdf(file);
}

#[test]
fn issue681() {
    let file = include_bytes!("../pdfs/load/issue681.pdf");
    load_pdf(file);
}

#[test]
fn issue682() {
    let file = include_bytes!("../pdfs/load/issue682.pdf");
    load_pdf(file);
}

#[test]
fn issue683() {
    let file = include_bytes!("../pdfs/load/issue683.pdf");
    load_pdf(file);
}

#[test]
fn issue676() {
    let file = include_bytes!("../pdfs/load/issue676.pdf");
    load_pdf(file);
}

#[test]
fn issue684() {
    let file = include_bytes!("../pdfs/load/issue684.pdf");
    load_pdf(file);
}

#[test]
fn issue705() {
    let file = include_bytes!("../pdfs/load/issue705.pdf");
    load_pdf(file);
}

#[test]
fn issue716() {
    let file = include_bytes!("../pdfs/load/issue716.pdf");
    load_pdf(file);
}

#[test]
fn image_offset_overflow() {
    let file = include_bytes!("../pdfs/load/image_offset_overflow.jp2");
    load_jpeg2000(file);
}

#[test]
fn unsupported_color_type() {
    let file = include_bytes!("../pdfs/load/unsupported_color_type.jp2");
    load_jpeg2000(file);
}

#[test]
fn different_resolution_levels() {
    let file = include_bytes!("../pdfs/load/different_resolution_levels.jp2");
    load_jpeg2000(file);
}

#[test]
fn large_layer_count() {
    let file = include_bytes!("../pdfs/load/large_layer_count.jp2");
    load_jpeg2000(file);
}

#[test]
fn too_many_coding_passes() {
    let file = include_bytes!("../pdfs/load/too_many_coding_passes.jp2");
    load_jpeg2000(file);
}

#[test]
fn precinct_overflow() {
    let file = include_bytes!("../pdfs/load/precinct_overflow.jp2");
    load_jpeg2000(file);
}

#[test]
fn metadata_in_object_stream() {
    // Normally, in an encrypted PDF file strings need to be encrypted when they are not
    // in a stream. Therefore, we need to ensure that no encryption is applied when the object
    // itself is in an object stream.
    let file = include_bytes!("../pdfs/custom/metadata_in_object_stream_encrypted.pdf");
    let pdf = Pdf::new(Arc::new(file.to_vec())).unwrap();

    let expected = Metadata {
        creation_date: Some(DateTime {
            year: 2025,
            month: 10,
            day: 26,
            hour: 16,
            minute: 24,
            second: 17,
            utc_offset_hour: 1,
            utc_offset_minute: 0,
        }),
        modification_date: Some(DateTime {
            year: 2025,
            month: 10,
            day: 26,
            hour: 16,
            minute: 33,
            second: 30,
            utc_offset_hour: 1,
            utc_offset_minute: 0,
        }),
        title: Some("Encrypted Metadata".as_bytes().to_vec()),
        author: Some("Max Mustermann".as_bytes().to_vec()),
        subject: None,
        keywords: None,
        creator: Some("Typst 0.14.0".as_bytes().to_vec()),
        producer: None,
    };

    assert_eq!(pdf.metadata(), &expected);
}
