//! Allocation behavior of page content arrays.

use hayro_syntax::Pdf;
use hayro_syntax::object::{ObjectIdentifier, Stream};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
    static FREED: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATED.set(ALLOCATED.get().wrapping_add(layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREED.set(FREED.get().wrapping_add(layout.size()));
        unsafe { System.dealloc(ptr, layout) }
    }
}

const CONTENTS: &str = "[4 0 R 5 0 R 7 0 R 6 0 R]";

fn document(contents: &str, ignored: &str, wrong_header: Option<&str>) -> Vec<u8> {
    let page = format!("<< /Type /Page /Parent 2 0 R /Contents {contents} >>");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        &page,
        "<< /Length 1 >>\nstream\nq\nendstream",
        ignored,
        "<< /Length 99 >>\nstream\nQ\nendstream",
    ];
    let mut pdf = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        if i == 3
            && let Some(header) = wrong_header
        {
            writeln!(pdf, "{header} obj\n<ab>\nendobj").unwrap();
        }
        writeln!(pdf, "{} 0 obj\n{body}\nendobj", i + 1).unwrap();
    }
    // Only a repaired xref can find object 7. Entry 8 points at its header.
    let unindexed = pdf.len();
    pdf.push_str("7 0 obj\n1\nendobj\n");
    let length = pdf.len();
    pdf.push_str("9 0 obj\n11\nendobj\n");
    let xref = pdf.len();
    pdf.push_str("xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(pdf, "8 2\n{unindexed:010} 00000 n \n{length:010} 00000 n ").unwrap();
    writeln!(
        pdf,
        "trailer\n<< /Size 10 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    )
    .unwrap();
    pdf.into_bytes()
}

fn read_ignoring(contents: &str, ignored: &str) -> (usize, usize) {
    let pdf = Pdf::new(document(contents, ignored, None)).unwrap();
    let page = &pdf.pages()[0];
    ALLOCATED.set(0);
    FREED.set(0);
    let content = page.page_stream().unwrap();
    let allocated = ALLOCATED.get();
    let retained = allocated - FREED.get();
    assert_eq!(content, b"q Q ");
    assert_eq!(pdf.xref().get::<u32>(ObjectIdentifier::new(7, 0)), None);
    (allocated, retained)
}

#[test]
fn ignored_content_string_cost_is_independent_of_length() {
    let read = |length| read_ignoring(CONTENTS, &format!("<{}>", "ab".repeat(length)));
    assert_eq!(read(16), read(1 << 20));
}

#[test]
fn ignored_content_dictionary_skips_keys_and_indirect_length() {
    let baseline = read_ignoring(CONTENTS, "<ab>");
    for suffix in ["", " streaming"] {
        for keys in [0, 1024] {
            let mut dict = String::from("<< /Length 8 0 R");
            for key in 0..keys {
                write!(dict, " /K{key} 1").unwrap();
            }
            dict.push_str(" >>");
            dict.push_str(suffix);
            assert_eq!(read_ignoring(CONTENTS, &dict), baseline);
        }
    }
}

#[test]
fn wrong_stream_offset_repairs_xref() {
    let pdf = Pdf::new(document(CONTENTS, "<ab>", Some("8 0"))).unwrap();
    assert_eq!(
        pdf.xref()
            .get::<Stream<'_>>(ObjectIdentifier::new(4, 0))
            .unwrap()
            .raw_data()
            .as_ref(),
        b"q"
    );
    assert_eq!(pdf.pages()[0].page_stream().unwrap(), b"q Q ");
    assert_eq!(pdf.xref().get::<u32>(ObjectIdentifier::new(7, 0)), Some(1));
}

#[test]
fn fractional_header_repairs_xref() {
    for header in ["4.5 0", "4 0.5"] {
        let bytes = document("4 0 R", "<ab>", Some(header));
        let pdf = Pdf::new(bytes.clone()).unwrap();
        assert_eq!(
            pdf.xref()
                .get::<Stream<'_>>(ObjectIdentifier::new(4, 0))
                .unwrap()
                .raw_data()
                .as_ref(),
            b"q"
        );
        let pdf = Pdf::new(bytes).unwrap();
        assert_eq!(pdf.pages()[0].page_stream(), Some(&b"q"[..]));
    }
}

#[test]
fn content_stream_honors_indirect_length_before_searching_for_endstream() {
    let stream = "<< /Length 9 0 R >> % comment\nstream\r\nxendstreamy\r\nendstream";
    let pdf = Pdf::new(document(CONTENTS, stream, None)).unwrap();
    assert_eq!(pdf.pages()[0].page_stream().unwrap(), b"q xendstreamy Q ");
}

#[test]
fn direct_content_string_cost_is_independent_of_length() {
    let read = |length| {
        let contents = format!("[4 0 R <{}> 6 0 R]", "41".repeat(length));
        read_ignoring(&contents, "<ab>")
    };
    assert_eq!(read(16), read(1 << 20));
}

#[test]
fn single_content_reference() {
    for (body, expected) in [
        ("<< /Length 1 >> stream\nQ\nendstream", Some(&b"Q"[..])),
        ("[4 0 R 6 0 R]", Some(&b"q Q "[..])),
        ("<< /Length 8 0 R >> streaming", None),
    ] {
        let pdf = Pdf::new(document("5 0 R", body, None)).unwrap();
        assert_eq!(pdf.pages()[0].page_stream(), expected);
        assert_eq!(pdf.xref().get::<u32>(ObjectIdentifier::new(7, 0)), None);
    }
}
