//! A predictor is reversed as `MuPDF` reverses it (observed in `PyMuPDF` 1.28.2 and
//! `MuPDF` 1.27.2): a last row cut short is kept, every row filter and pixel size is
//! unfiltered a byte at a time, and TIFF prediction works per component, rows that end
//! inside a byte included.

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
    // PNG, row filter 0, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[0, 10, 20, 30, 40, 50, 0],
        &[10, 20, 30, 40, 50],
    ),
    // PNG, row filter 0, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[0, 10, 20, 30, 40, 50, 0, 1],
        &[10, 20, 30, 40, 50, 1],
    ),
    // PNG, row filter 0, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[0, 10, 20, 30, 40, 50, 0, 1, 2, 3],
        &[10, 20, 30, 40, 50, 1, 2, 3],
    ),
    // PNG, row filter 1, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[1, 10, 20, 30, 40, 50, 1],
        &[10, 30, 60, 100, 150],
    ),
    // PNG, row filter 1, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[1, 10, 20, 30, 40, 50, 1, 1],
        &[10, 30, 60, 100, 150, 1],
    ),
    // PNG, row filter 1, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[1, 10, 20, 30, 40, 50, 1, 1, 2, 3],
        &[10, 30, 60, 100, 150, 1, 3, 6],
    ),
    // PNG, row filter 2, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[2, 10, 20, 30, 40, 50, 2],
        &[10, 20, 30, 40, 50],
    ),
    // PNG, row filter 2, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[2, 10, 20, 30, 40, 50, 2, 1],
        &[10, 20, 30, 40, 50, 11],
    ),
    // PNG, row filter 2, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[2, 10, 20, 30, 40, 50, 2, 1, 2, 3],
        &[10, 20, 30, 40, 50, 11, 22, 33],
    ),
    // PNG, row filter 3, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[3, 10, 20, 30, 40, 50, 3],
        &[10, 25, 42, 61, 80],
    ),
    // PNG, row filter 3, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[3, 10, 20, 30, 40, 50, 3, 1],
        &[10, 25, 42, 61, 80, 6],
    ),
    // PNG, row filter 3, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[3, 10, 20, 30, 40, 50, 3, 1, 2, 3],
        &[10, 25, 42, 61, 80, 6, 17, 32],
    ),
    // PNG, row filter 4, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[4, 10, 20, 30, 40, 50, 4],
        &[10, 30, 60, 100, 150],
    ),
    // PNG, row filter 4, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[4, 10, 20, 30, 40, 50, 4, 1],
        &[10, 30, 60, 100, 150, 11],
    ),
    // PNG, row filter 4, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[4, 10, 20, 30, 40, 50, 4, 1, 2, 3],
        &[10, 30, 60, 100, 150, 11, 32, 63],
    ),
    // PNG, row filter 7, a last row of 0 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[7, 10, 20, 30, 40, 50, 7],
        &[10, 20, 30, 40, 50],
    ),
    // PNG, row filter 7, a last row of 1 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[7, 10, 20, 30, 40, 50, 7, 1],
        &[10, 20, 30, 40, 50, 1],
    ),
    // PNG, row filter 7, a last row of 3 of 5 bytes.
    (
        "/Predictor 12 /Columns 5",
        &[7, 10, 20, 30, 40, 50, 7, 1, 2, 3],
        &[10, 20, 30, 40, 50, 1, 2, 3],
    ),
    // PNG, 3-byte pixels, row filter 1, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 3 /Columns 2",
        &[1, 81, 192, 221, 143, 237, 163, 1, 176, 163, 127, 28],
        &[81, 192, 221, 224, 173, 128, 176, 163, 127, 204],
    ),
    // PNG, 3-byte pixels, row filter 3, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 3 /Columns 2",
        &[3, 247, 207, 98, 246, 11, 1, 3, 222, 57, 6, 47],
        &[247, 207, 98, 113, 114, 50, 89, 160, 55, 148],
    ),
    // PNG, 3-byte pixels, row filter 4, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 3 /Columns 2",
        &[4, 68, 23, 144, 28, 231, 16, 4, 128, 225, 227, 221],
        &[68, 23, 144, 96, 254, 160, 196, 248, 115, 161],
    ),
    // PNG, 5-byte pixels, row filter 1, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 5 /Columns 2",
        &[
            1, 81, 192, 221, 143, 237, 163, 66, 47, 105, 197, 2, 162, 132, 177, 192, 144, 228, 104,
            167, 7, 237, 1, 176, 163, 127, 28, 190, 55, 235,
        ],
        &[
            81, 192, 221, 143, 237, 244, 2, 12, 248, 178, 243, 68, 142, 79, 125, 216, 106, 179,
            255, 159, 176, 163, 127, 28, 190, 231, 142,
        ],
    ),
    // PNG, 5-byte pixels, row filter 2, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 5 /Columns 2",
        &[
            2, 162, 132, 177, 192, 144, 228, 104, 167, 7, 237, 3, 247, 207, 98, 246, 11, 1, 77,
            247, 228, 235, 2, 237, 237, 159, 98, 91, 176, 22,
        ],
        &[
            162, 132, 177, 192, 144, 228, 104, 167, 7, 237, 72, 17, 186, 86, 83, 151, 137, 167, 18,
            139, 53, 254, 89, 184, 174, 71, 159,
        ],
    ),
    // PNG, 5-byte pixels, row filter 3, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 5 /Columns 2",
        &[
            3, 247, 207, 98, 246, 11, 1, 77, 247, 228, 235, 4, 68, 23, 144, 28, 231, 16, 237, 22,
            248, 78, 3, 222, 57, 6, 47, 103, 111, 154,
        ],
        &[
            247, 207, 98, 246, 11, 124, 180, 40, 95, 240, 59, 230, 242, 18, 242, 75, 188, 8, 10,
            64, 251, 172, 127, 56, 224, 18, 78,
        ],
    ),
    // PNG, 5-byte pixels, row filter 4, a last row cut inside a pixel.
    (
        "/Predictor 12 /Colors 5 /Columns 2",
        &[
            4, 68, 23, 144, 28, 231, 16, 237, 22, 248, 78, 0, 216, 90, 153, 23, 100, 71, 51, 255,
            221, 101, 4, 128, 225, 227, 221, 167, 246, 248,
        ],
        &[
            68, 23, 144, 28, 231, 84, 4, 166, 20, 53, 216, 90, 153, 23, 100, 71, 51, 255, 221, 101,
            88, 59, 124, 244, 11, 61, 43,
        ],
    ),
    // PNG, 16-bit components, 6-byte pixels, a last row cut inside a pixel.
    (
        "/Predictor 15 /Colors 3 /BitsPerComponent 16 /Columns 1",
        &[4, 56, 136, 121, 130, 118, 134, 4, 37, 246, 136, 133, 103],
        &[56, 136, 121, 130, 118, 134, 93, 126, 1, 7, 221],
    ),
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 1.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[
            1, 48, 232, 166, 56, 98, 1, 122, 14, 0, 30, 61, 1, 214, 232, 157,
        ],
        &[48, 232, 214, 32, 56, 122, 14, 122, 44, 183, 214, 232, 115],
    ),
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 3.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[
            3, 21, 97, 79, 15, 41, 3, 174, 115, 80, 120, 93, 3, 15, 54, 105,
        ],
        &[21, 97, 89, 63, 85, 184, 163, 216, 233, 243, 107, 135, 10],
    ),
    // PNG, rows of 5 bytes and 2-byte pixels, row filter 4.
    (
        "/Predictor 12 /Colors 3 /BitsPerComponent 4 /Columns 3",
        &[
            4, 180, 149, 65, 114, 38, 4, 113, 121, 99, 182, 83, 4, 74, 225, 33,
        ],
        &[180, 149, 245, 7, 27, 37, 14, 136, 189, 110, 111, 239, 169],
    ),
    // TIFF, 8-bit components, 3 colors, a last row cut inside a pixel.
    (
        "/Predictor 2 /Colors 3 /Columns 2",
        &[87, 137, 138, 38, 189, 63, 149, 100, 41, 6],
        &[87, 137, 138, 125, 70, 201, 149, 100, 41, 155],
    ),
    // TIFF, 8-bit components, 5 colors, a last row cut inside a pixel.
    (
        "/Predictor 2 /Colors 5 /Columns 2",
        &[
            44, 24, 195, 29, 211, 20, 213, 157, 253, 38, 92, 82, 253, 205, 175, 86, 117,
        ],
        &[
            44, 24, 195, 29, 211, 64, 237, 96, 26, 249, 92, 82, 253, 205, 175, 178, 199,
        ],
    ),
    // TIFF, 16-bit components, a last row cut inside a component: its low byte is the one the row before held there, as stored.
    (
        "/Predictor 2 /BitsPerComponent 16 /Columns 3",
        &[10, 20, 30, 40, 50, 60, 1, 2, 3, 132, 53],
        &[10, 20, 40, 60, 90, 120, 1, 2, 4, 134, 57],
    ),
    // TIFF, 16-bit components, 2 colors, a last row cut inside a component.
    (
        "/Predictor 2 /BitsPerComponent 16 /Colors 2 /Columns 2",
        &[
            172, 87, 147, 139, 240, 208, 231, 52, 157, 150, 191, 102, 233, 93, 50, 126, 113, 56,
            94, 33, 35, 42, 117,
        ],
        &[
            172, 87, 147, 139, 157, 39, 122, 191, 157, 150, 191, 102, 134, 243, 241, 228, 113, 56,
            94, 33, 148, 98, 211,
        ],
    ),
    // TIFF, 16-bit components, a last row cut inside a component whose missing low byte carries:
    // the byte the row before held there as stored, not as decoded nor zero.
    (
        "/Predictor 2 /BitsPerComponent 16 /Columns 2",
        &[0, 1, 0, 255, 0, 1, 5],
        &[0, 1, 1, 0, 0, 1, 6],
    ),
    // TIFF, 4-bit components, rows of 11: the bits after the last are cleared.
    (
        "/Predictor 2 /BitsPerComponent 4 /Columns 11",
        &[
            216, 160, 99, 170, 79, 202, 5, 190, 189, 226, 66, 224, 87, 126, 155, 19,
        ],
        &[
            213, 255, 88, 44, 15, 176, 5, 14, 150, 70, 172, 160, 92, 49, 165, 105,
        ],
    ),
    // TIFF, 1-bit components, rows of 45.
    (
        "/Predictor 2 /BitsPerComponent 1 /Columns 45",
        &[134, 8, 5, 129, 237, 40, 74, 220, 123, 251, 98, 136, 96, 213],
        &[
            251, 240, 6, 254, 182, 48, 115, 104, 82, 173, 188, 240, 64, 153,
        ],
    ),
    // TIFF, 2-bit components, 3 colors, rows of 18 bits.
    (
        "/Predictor 2 /BitsPerComponent 2 /Colors 3 /Columns 3",
        &[107, 103, 197, 105, 133, 168, 118, 26],
        &[104, 198, 192, 106, 45, 0, 119, 229],
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
    // A row far longer than the data is a first row cut short.
    (
        "/Predictor 12 /Columns 100000000",
        &[1, 125, 243, 136, 28, 230, 25, 77, 184, 94, 254, 221, 108],
        &[125, 112, 248, 20, 250, 19, 96, 24, 118, 116, 81, 189],
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

/// A content under a predictor whose last row is cut short keeps that row, as `MuPDF`
/// draws it.
#[test]
fn a_content_row_cut_short_is_kept() {
    let content = common::cut_row_content_stream();
    let pdf = Pdf::new(common::one_page(100, "/Contents 4 0 R", vec![content])).unwrap();
    let page = &pdf.pages()[0];
    assert_eq!(page.page_stream().unwrap(), common::CUT_ROW_CONTENT);
}
