//! Allocation bounds for overlapping progression order changes.

use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

#[derive(Clone, Copy, Default)]
struct Allocations {
    live: isize,
    peak: isize,
    total: usize,
}

thread_local! {
    static MEASUREMENT: Cell<Option<Allocations>> = const { Cell::new(None) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null()
            && let Some(mut counts) = MEASUREMENT.get()
        {
            counts.live += layout.size() as isize;
            counts.peak = counts.peak.max(counts.live);
            counts.total += layout.size();
            MEASUREMENT.set(Some(counts));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(mut counts) = MEASUREMENT.get() {
            counts.live -= layout.size() as isize;
            MEASUREMENT.set(Some(counts));
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn decode(data: &[u8]) -> (Vec<u8>, isize, usize) {
    MEASUREMENT.set(Some(Allocations::default()));
    let image = Image::new(data, &DecodeSettings::default()).unwrap();
    let mut ctx = DecoderContext::default();
    let samples = image.decode(&mut ctx).unwrap().data_u8().unwrap();
    let counts = MEASUREMENT.take().unwrap();
    (samples, counts.peak, counts.total)
}

fn marker(code: u16, body: &[u8]) -> Vec<u8> {
    let mut marker = code.to_be_bytes().to_vec();
    marker.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    marker.extend(body);
    marker
}

fn one_row(width: u32, layer_ends: &[u8]) -> Vec<u8> {
    let layers = *layer_ends.last().unwrap();
    let mut siz = vec![0, 0];
    for value in [width, 1, 0, 0, width, 1, 0, 0] {
        siz.extend(value.to_be_bytes());
    }
    siz.extend([0, 1, 7, 1, 1]);
    let cod = [1, 2, 0, layers, 0, 0, 4, 4, 0, 1, 0];
    let qcd = [0x40, 0x48];
    let poc: Vec<_> = layer_ends
        .iter()
        .flat_map(|&end| [0, 0, 0, end, 1, 1, 2])
        .collect();
    let data = vec![0; width as usize * usize::from(layers) + 1];
    let tile_part_len = u32::try_from(12 + 2 + data.len()).unwrap();
    let mut sot = vec![0, 0];
    sot.extend(tile_part_len.to_be_bytes());
    sot.extend([0, 1]);

    let mut stream = vec![0xFF, 0x4F];
    stream.extend(marker(0xFF51, &siz));
    stream.extend(marker(0xFF52, &cod));
    stream.extend(marker(0xFF5C, &qcd));
    stream.extend(marker(0xFF5F, &poc));
    stream.extend(marker(0xFF90, &sot));
    stream.extend([0xFF, 0x93]);
    stream.extend(data);
    stream.extend([0xFF, 0xD9]);
    stream
}

#[test]
fn repeated_changes_are_skipped() {
    let (expected, once_peak, once) = decode(&one_row(1000, &[1]));
    let (actual, many_peak, many) = decode(&one_row(1000, &[1; 1000]));
    assert!(actual == expected, "the image changed");
    assert!(
        many_peak < once_peak + once_peak / 2,
        "peak grew from {once_peak} to {many_peak} bytes"
    );
    assert!(
        many < 2 * once,
        "1000 changes allocated {many} bytes in total, against {once} for one"
    );
}

#[test]
fn consecutive_changes_release_the_previous_list() {
    let width = 4096;
    let (expected, once, _) = decode(&one_row(width, &[2, 2]));
    let (actual, split, _) = decode(&one_row(width, &[1, 2]));
    assert!(actual == expected, "the image changed");
    assert!(
        split < once + width as isize,
        "two changes peaked at {split} bytes, against {once} for one"
    );
}
