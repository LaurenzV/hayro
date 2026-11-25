//! Regression tests for skipping unusable `/ObjStm` headers during xref rebuild.

use hayro_syntax::Pdf;
use hayro_syntax::object::ObjectIdentifier;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAlloc;

static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            ALLOCATED_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn rebuild_pdf_with_objstm(header: &str, run_length_repeats: usize) -> Vec<u8> {
    // Each RunLengthDecode pair [129, b'A'] expands to 128 bytes of 'A'.
    let mut payload = Vec::with_capacity(run_length_repeats * 2 + 1);
    for _ in 0..run_length_repeats {
        payload.push(129);
        payload.push(b'A');
    }
    payload.push(128); // EOD

    let mut pdf = Vec::new();
    pdf.extend_from_slice(
        b"%PDF-1.5\n\
          1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
          2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
          3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>\nendobj\n",
    );
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /ObjStm {header} /Filter /RunLengthDecode /Length {} >>\nstream\n",
            payload.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&payload);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n999999999\n%%EOF\n");
    pdf
}

#[test]
fn rebuild_skips_decoding_unusable_objstm_headers() {
    let _guard = TEST_LOCK.lock().unwrap();
    // 2,048 repeats × 128 B = 256 KiB decoded output if RunLengthDecode runs.
    for header in [
        "/N 0 /First 0",
        "/N -1 /First 0",
        "/First 0",
        "/N 1",
        "/N 1 /First -1",
        "/N 1 /First /Invalid",
    ] {
        let bytes = rebuild_pdf_with_objstm(header, 2_048);
        ALLOCATED_BYTES.store(0, Ordering::Relaxed);
        let pdf = Pdf::new(bytes).unwrap();
        let allocated = ALLOCATED_BYTES.load(Ordering::Relaxed);
        assert_eq!(pdf.pages().len(), 1);
        assert!(
            allocated < 64 * 1024,
            "header `{header}` decoded stream during rebuild ({allocated} bytes allocated)"
        );
    }

    // Compressed-root trailer validation must also skip an overwritten /N 0 stream.
    let mut payload = Vec::with_capacity(2_048 * 2 + 1);
    for _ in 0..2_048 {
        payload.push(129);
        payload.push(b'A');
    }
    payload.push(128);
    let mut pdf_bytes = b"%PDF-1.5\n\
        1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
        2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
        3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>\nendobj\n\
        4 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length 6 >>\nstream\n6 0 42\nendstream\nendobj\n"
        .to_vec();
    pdf_bytes.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /ObjStm /N 0 /First 0 /Filter /RunLengthDecode /Length {} >>\nstream\n",
            payload.len()
        )
        .as_bytes(),
    );
    pdf_bytes.extend_from_slice(&payload);
    pdf_bytes.extend_from_slice(
        b"\nendstream\nendobj\n\
          trailer\n<< /Size 7 /Root 6 0 R >>\n\
          trailer\n<< /Size 7 /Root 1 0 R >>\n\
          startxref\n999999999\n%%EOF\n",
    );
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    let pdf = Pdf::new(pdf_bytes).unwrap();
    let allocated = ALLOCATED_BYTES.load(Ordering::Relaxed);
    assert_eq!(pdf.pages().len(), 1);
    assert!(
        allocated < 64 * 1024,
        "trailer validation decoded overwritten /N 0 stream ({allocated} bytes allocated)"
    );
}

#[test]
fn rebuild_still_recovers_usable_objstm_and_normal_lookup_still_repairs() {
    let _guard = TEST_LOCK.lock().unwrap();
    // 1. Rebuild recovers compressed object 6 = 42 from valid /N 1 /First 4 stream.
    let rebuild_bytes = b"%PDF-1.5\n\
        1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
        2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
        3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>\nendobj\n\
        4 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Filter /RunLengthDecode /Length 8 >>\nstream\n\x056 0 42\x80\nendstream\nendobj\n\
        trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n999999999\n%%EOF\n";
    let pdf = Pdf::new(rebuild_bytes.to_vec()).unwrap();
    assert_eq!(pdf.xref().get::<u32>(ObjectIdentifier::new(6, 0)), Some(42));

    // 2. Normal xref pointing object 6 at a decodable /N 0 stream still triggers repair
    //    and recovers standalone object 6 = 99.
    let mut pdf_bytes = b"%PDF-1.5\n".to_vec();
    let mut offsets = [0_usize; 7];
    let mut push_obj = |id: usize, body: &[u8]| {
        offsets[id] = pdf_bytes.len();
        pdf_bytes.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        pdf_bytes.extend_from_slice(body);
        pdf_bytes.extend_from_slice(b"\nendobj\n");
    };
    push_obj(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    push_obj(2, b"<< /Type /Pages /Kids [] /Count 0 >>");
    push_obj(
        5,
        b"<< /Type /ObjStm /N 0 /First 0 /Length 0 >>\nstream\n\nendstream",
    );
    push_obj(6, b"99");

    let mut entries = Vec::new();
    let push_entry = |buf: &mut Vec<u8>, typ: u8, f2: u32, f3: u8| {
        buf.push(typ);
        buf.extend_from_slice(&f2.to_be_bytes()[2..]);
        buf.push(f3);
    };
    push_entry(&mut entries, 0, 0, 255); // 0
    push_entry(&mut entries, 1, offsets[1] as u32, 0); // 1
    push_entry(&mut entries, 1, offsets[2] as u32, 0); // 2
    push_entry(&mut entries, 0, 0, 0); // 3
    push_entry(&mut entries, 0, 0, 0); // 4
    push_entry(&mut entries, 1, offsets[5] as u32, 0); // 5 -> decodable /N 0 stream
    push_entry(&mut entries, 2, 5, 0); // 6 -> stale compressed entry pointing into 5

    let xref_offset = pdf_bytes.len();
    push_entry(&mut entries, 1, xref_offset as u32, 0); // 7 -> xref stream itself
    pdf_bytes.extend_from_slice(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /Root 1 0 R /W [1 2 1] /Length {} >>\nstream\n",
            entries.len()
        )
        .as_bytes(),
    );
    pdf_bytes.extend_from_slice(&entries);
    pdf_bytes.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );

    let pdf = Pdf::new(pdf_bytes).unwrap();
    // Original xref has no entry for object 7's replacement; entry 6 triggers lazy repair.
    assert_eq!(pdf.xref().get::<u32>(ObjectIdentifier::new(6, 0)), Some(99));
}
