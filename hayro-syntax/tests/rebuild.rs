//! Resource bounds for xref recovery.
use hayro_syntax::Pdf;
use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

const STREAM_LEN: usize = 65_521;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct StreamAllocations;

#[global_allocator]
static ALLOCATOR: StreamAllocations = StreamAllocations;

unsafe impl GlobalAlloc for StreamAllocations {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if layout.size() == STREAM_LEN && !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            PEAK.fetch_max(LIVE.fetch_add(1, Relaxed) + 1, Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() == STREAM_LEN {
            LIVE.fetch_sub(1, Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[test]
fn interleaved_compressed_roots_decode_each_stream_once_per_pass() {
    let counts = [2, 200].map(|trailers| {
        let mut pdf = String::from(
            "1 0 obj << /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n",
        );
        for stream in 3..=4 {
            let mut header = String::new();
            for i in 0..100 {
                write!(header, "{} {} ", 10 + 2 * i + stream - 3, 5 * i).unwrap();
            }
            writeln!(
                pdf,
                "{stream} 0 obj << /Type /ObjStm /N 100 /First {} /Length {STREAM_LEN} >>\nstream",
                header.len()
            )
            .unwrap();
            let start = pdf.len();
            pdf.push_str(&header);
            pdf.push_str(&"null ".repeat(100));
            pdf.extend(std::iter::repeat_n(' ', STREAM_LEN - (pdf.len() - start)));
            pdf.push_str("\nendstream\nendobj\n");
        }
        pdf.push_str("trailer << /Root 1 0 R >>\n");
        for i in 0..trailers {
            writeln!(pdf, "trailer << /Root {} 0 R >>", 10 + i).unwrap();
        }

        ALLOCATIONS.store(0, Relaxed);
        PEAK.store(0, Relaxed);
        Pdf::new(pdf.into_bytes()).unwrap();
        (ALLOCATIONS.load(Relaxed), PEAK.load(Relaxed))
    });
    assert_eq!(counts, [(4, 1); 2]);
}
