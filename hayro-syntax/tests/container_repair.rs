//! Tests for reconstructing cross-references in damaged PDFs.

use hayro_syntax::object::{Dict, ObjectIdentifier};
use hayro_syntax::{Pdf, PdfData};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAlloc;

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
static MAX_ALLOC: AtomicUsize = AtomicUsize::new(0);
static ALLOC_GATE: RwLock<()> = RwLock::new(());

fn record_alloc(size: usize) {
    if TRACKING.load(Ordering::Relaxed) {
        ALLOC_BYTES.fetch_add(size, Ordering::Relaxed);
        MAX_ALLOC.fetch_max(size, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_alloc(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_alloc(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_alloc(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

struct TrackingGuard;

impl TrackingGuard {
    fn start() -> Self {
        ALLOC_BYTES.store(0, Ordering::Relaxed);
        MAX_ALLOC.store(0, Ordering::Relaxed);
        TRACKING.store(true, Ordering::Release);
        Self
    }
}

impl Drop for TrackingGuard {
    fn drop(&mut self) {
        TRACKING.store(false, Ordering::Release);
    }
}

#[test]
fn repair_finds_catalog_inside_broken_container() {
    let _guard = ALLOC_GATE.read().unwrap();
    let padded_child = format!("[{}] ", " ".repeat(64));
    let obj_adj_array = (1..=64).map(|i| format!("{i} 0 obj[ ")).collect::<String>();
    let obj_adj_dict = (1..=64)
        .map(|i| format!("{i} 0 obj<< "))
        .collect::<String>();

    for broken in [
        "<< /A <<",
        "<< /A [",
        "<< /A [ 1 1 1 1",
        "[ 5 0 obj [",
        &format!("[ {obj_adj_array}"),
        &format!("[ {obj_adj_dict}"),
        &format!("[ {}", "% 1 0\nobj [ ".repeat(1024)),
        &format!("[ {}", "% 1 0\nobj<< ".repeat(1024)),
        &format!("[ {}", "( 1 0 obj %)\n[ ".repeat(1024)),
        &format!("[ {}\n0 {}", "1 0 obj [% ".repeat(256), "[ ".repeat(16_384)),
        &format!("{}{}", "[".repeat(512), "[]".repeat(10_000)),
        &format!(
            "{}%{}\n{}",
            "[".repeat(513),
            "x".repeat(65536),
            "]".repeat(513)
        ),
        &"[ ".repeat(2000),
        &"<<".repeat(2000),
        &format!("[ [ {}]", "[] ".repeat(5000)),
        &format!("[ [ {}]", padded_child.repeat(500)),
        &format!("[ [ {}]", "<< >> ".repeat(5000)),
        &format!("{}{}", "[".repeat(2000), "]".repeat(2000)),
    ] {
        for trailer in ["", "trailer\n<< /Root 700 0 R >>\n"] {
            let data = format!(
                "%PDF-1.4\n100 0 obj {broken}\n\
            700 0 obj << /Type /Catalog /Pages 800 0 R >> endobj\n\
            800 0 obj << /Type /Pages /Kids [900 0 R] /Count 1 >> endobj\n\
            900 0 obj << /Type /Page /Parent 800 0 R /MediaBox [0 0 72 72] >> endobj\n\
            {trailer}%%EOF"
            );
            let pdf = Pdf::new(data.into_bytes()).unwrap();
            assert_eq!(pdf.xref().root_id(), ObjectIdentifier::new(700, 0));
            for id in [700, 800, 900] {
                assert!(
                    pdf.xref()
                        .get::<Dict<'_>>(ObjectIdentifier::new(id, 0))
                        .is_some()
                );
            }
            assert_eq!(pdf.pages().len(), 1);
            assert_eq!(pdf.len(), 3, "prefix {:?}", &broken[..broken.len().min(48)]);
        }
    }
}

#[test]
fn repair_skips_huge_escaped_key_without_allocating_key_buffer() {
    let huge = format!("#41{}", "A".repeat(1024 * 1024));
    let data = format!(
        "%PDF-1.4\n\
        1 0 obj << /{huge} 1 >> endobj\n\
        7 0 obj << /Type /Catalog /Pages 8 0 R >> endobj\n\
        8 0 obj << /Type /Pages /Kids [9 0 R] /Count 1 >> endobj\n\
        9 0 obj << /Type /Page /Parent 8 0 R /MediaBox [0 0 72 72] >> endobj\n\
        %%EOF"
    );
    let pdf_data = PdfData::from(data.into_bytes());

    let _guard = ALLOC_GATE.write().unwrap();
    let pdf = {
        let _tracking = TrackingGuard::start();
        Pdf::new(pdf_data).unwrap()
    };
    let alloc_bytes = ALLOC_BYTES.load(Ordering::Acquire);
    let max_alloc = MAX_ALLOC.load(Ordering::Acquire);

    assert_eq!(pdf.xref().root_id(), ObjectIdentifier::new(7, 0));
    assert_eq!(pdf.len(), 4);
    assert!(
        alloc_bytes < 64 * 1024,
        "expected < 64 KiB allocated during repair of 1 MiB escaped key, got {alloc_bytes} bytes (max single alloc {max_alloc} bytes)"
    );
    assert!(
        max_alloc < 64 * 1024,
        "expected max single allocation < 64 KiB, got {max_alloc} bytes (total {alloc_bytes} bytes)"
    );
}
