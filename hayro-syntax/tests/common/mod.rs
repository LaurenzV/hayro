//! Small PDF files and the data in them, for tests of this crate and of hayro.

/// A stream object with `dict` entries besides `/Length`.
pub(crate) fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

/// A PDF file whose objects are numbered from 1 in this order, object 1 being the
/// catalog.
pub(crate) fn file(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let size = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// `data` in a zlib stream of stored (uncompressed) deflate blocks of at most `block`
/// bytes, which every Flate decoder reads without a compressor.
pub(crate) fn stored_zlib(data: &[u8], block: usize) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(block).peekable();
    if blocks.peek().is_none() {
        out.extend([1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(b) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let len = b.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend_from_slice(b);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for &d in data {
        a = (a + u32::from(d)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

/// A file with one `size` × `size` pt page with `entries` besides its type, parent and
/// media box (objects 1 to 3: catalog, page tree, page), and `objects` numbered from 4.
pub(crate) fn one_page(size: u32, entries: &str, objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {size} {size}] {entries} >>")
            .into_bytes(),
    ];
    all.extend(objects);
    file(&all)
}

/// A page content of 26 bytes that fills a blue square, `0 0 1 rg 10 10 50 50 re  f`.
pub(crate) const CUT_ROW_CONTENT: &[u8] = b"0 0 1 rg 10 10 50 50 re  f";

/// The content stream of [`CUT_ROW_CONTENT`] in Flate under the PNG Up predictor in
/// rows of 5, its last row the `f` alone, which `MuPDF` keeps and draws (`PyMuPDF`
/// 1.28.2).
pub(crate) fn cut_row_content_stream() -> Vec<u8> {
    let mut rows = Vec::new();
    let mut above = [0_u8; 5];
    for row in CUT_ROW_CONTENT.chunks(5) {
        rows.push(2);
        rows.extend(row.iter().zip(above).map(|(b, a)| b.wrapping_sub(a)));
        above[..row.len()].copy_from_slice(row);
    }
    stream(
        "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 5 >>",
        &stored_zlib(&rows, 64),
    )
}
