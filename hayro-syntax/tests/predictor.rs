//! A predictor is reversed as `MuPDF` reverses it (observed in `PyMuPDF` 1.28.2 and
//! `MuPDF` 1.27.2): every row filter and pixel size is unfiltered a byte at a time, and
//! TIFF prediction works per component, rows that end inside a byte included.

use hayro_syntax::Pdf;
use hayro_syntax::object::{ObjectIdentifier, Stream};

mod common;

/// A document whose object 3 is a stream with `dict` entries holding `data`.
fn stream_file(dict: &str, data: &[u8]) -> Pdf {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        common::stream(dict, data),
    ];
    Pdf::new(common::file(&objects)).unwrap()
}

fn object_3(pdf: &Pdf) -> Stream<'_> {
    pdf.xref().get(ObjectIdentifier::new(3, 0)).unwrap()
}

/// `data` LZW-encoded in 9-bit codes, which suffice for short data: a clear-table
/// code, each byte, the end-of-data code.
fn lzw(data: &[u8]) -> Vec<u8> {
    let codes = [256].into_iter().chain(data.iter().map(|&b| u32::from(b)));
    let (mut bits, mut count, mut out) = (0_u64, 0, Vec::new());
    for code in codes.chain([257]) {
        bits = bits << 9 | u64::from(code);
        count += 9;
        while count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    out.push((bits << (8 - count)) as u8);
    out
}

/// The decode parameters, the rows as stored before compression, and what `MuPDF`
/// decodes them to (`PyMuPDF`'s `Document::xref_stream`).
const CASES: &[(&str, &[u8], &[u8])] = &[
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 1.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[1, 48, 232, 166, 56, 98, 1, 122, 14, 0, 30, 61],
        &[48, 232, 214, 32, 56, 122, 14, 122, 44, 183],
    ),
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 3.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[3, 21, 97, 79, 15, 41, 3, 174, 115, 80, 120, 93],
        &[21, 97, 89, 63, 85, 184, 163, 216, 233, 243],
    ),
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 4.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[4, 180, 149, 65, 114, 38, 4, 113, 121, 99, 182, 83],
        &[180, 149, 245, 7, 27, 37, 14, 136, 189, 110],
    ),
    // TIFF, 4-bit components, rows of 11: the bits after the last are cleared.
    (
        "/Predictor 2 /BitsPerComponent 4 /Columns 11",
        &[216, 160, 99, 170, 79, 202, 5, 190, 189, 226, 66, 224],
        &[213, 255, 88, 44, 15, 176, 5, 14, 150, 70, 172, 160],
    ),
    // TIFF, 1-bit components, rows of 45.
    (
        "/Predictor 2 /BitsPerComponent 1 /Columns 45",
        &[134, 8, 5, 129, 237, 40, 74, 220, 123, 251, 98, 136],
        &[251, 240, 6, 254, 182, 48, 115, 104, 82, 173, 188, 240],
    ),
    // TIFF, 2-bit components, 3 colors, rows of 18 bits.
    (
        "/Predictor 2 /BitsPerComponent 2 /Colors 3 /Columns 3",
        &[107, 103, 197, 105, 133, 168],
        &[104, 198, 192, 106, 45, 0],
    ),
    // TIFF, rows of 6 bits: the rows after the first are not lost.
    (
        "/Predictor 2 /BitsPerComponent 2 /Colors 3 /Columns 1",
        &[108, 108, 108],
        &[108, 108, 108],
    ),
    // TIFF, rows of 12 bits.
    (
        "/Predictor 2 /BitsPerComponent 4 /Colors 3 /Columns 1",
        &[18, 48, 18, 48, 18, 48],
        &[18, 48, 18, 48, 18, 48],
    ),
];

/// Each case decodes to `MuPDF`'s bytes, compressed by Flate and by LZW, under
/// `/DecodeParms` or `/DP`.
#[test]
fn predicted_rows_decode_as_mupdf_decodes_them() {
    for (parms, rows, expected) in CASES {
        for (filter, encoded) in [
            ("/FlateDecode", common::stored_zlib(rows, 64)),
            ("/LZWDecode", lzw(rows)),
        ] {
            for key in ["/DecodeParms", "/DP"] {
                let dict = format!("/Filter {filter} {key} << {parms} >>");
                let pdf = stream_file(&dict, &encoded);
                let decoded = object_3(&pdf).decoded().ok();
                assert_eq!(decoded.as_deref(), Some(*expected), "{dict}");
            }
        }
    }
}

/// Parameters whose row length overflows the integers it used to be computed in: decoding
/// may fail, but must not panic.
#[test]
fn predictor_parameters_that_overflow_do_not_panic() {
    for parms in [
        "/Predictor 2 /Colors 65 /BitsPerComponent 4 /Columns 2",
        "/Predictor 12 /Colors 255 /BitsPerComponent 16 /Columns 3",
        "/Predictor 2 /Colors 255 /BitsPerComponent 16 /Columns 9223372036854775807",
    ] {
        let dict = format!("/Filter /LZWDecode /DecodeParms << {parms} >>");
        let pdf = stream_file(&dict, &lzw(&[0x12, 0x34, 0x56]));
        let _ = object_3(&pdf).decoded();
    }
}
