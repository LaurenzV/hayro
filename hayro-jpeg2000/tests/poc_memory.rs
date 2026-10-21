//! Memory and work of overlapping progression order changes (POC, A.6.6), in a
//! binary of its own as it counts every allocation.

use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
// Tests run in parallel threads, and the counters are shared.
static COUNTING: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The decoded samples, the peak of memory allocated while decoding and the
/// total of all allocations.
fn decode(data: &[u8]) -> (Vec<u8>, usize, usize) {
    let _counting = COUNTING.lock().unwrap();
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let total = TOTAL.load(Ordering::Relaxed);
    let image = Image::new(data, &DecodeSettings::default()).unwrap();
    let mut ctx = DecoderContext::default();
    let samples = image.decode(&mut ctx).unwrap().data_u8();
    (
        samples,
        PEAK.load(Ordering::Relaxed) - base,
        TOTAL.load(Ordering::Relaxed) - total,
    )
}

fn marker(code: u16, body: &[u8]) -> Vec<u8> {
    let mut marker = code.to_be_bytes().to_vec();
    marker.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    marker.extend(body);
    marker
}

/// A grayscale image of `width` × 1 pixels in one tile, with one layer, no
/// decomposition, 1 × 1 precincts and `changes` RPCL changes over all of it.
/// Every packet is empty, and one more byte follows them.
fn one_row(width: u32, changes: usize) -> Vec<u8> {
    let mut siz = vec![0, 0];
    for value in [width, 1, 0, 0, width, 1, 0, 0] {
        siz.extend(value.to_be_bytes());
    }
    siz.extend([0, 1, 7, 1, 1]);
    let cod = [1, 2, 0, 1, 0, 0, 4, 4, 0, 1, 0];
    let qcd = [0x40, 0x48];
    let poc = [0, 0, 0, 1, 1, 1, 2].repeat(changes);
    let data = vec![0; width as usize + 1];
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
    // The extra byte makes the decoder ask for one more packet after the
    // first change, so every other change is reached. Each lists the 1000
    // precincts again unless it is skipped.
    let (expected, _, once) = decode(&one_row(1000, 1));
    let (actual, _, many) = decode(&one_row(1000, 1000));
    assert!(actual == expected, "the image changed");
    assert!(
        many < 2 * once,
        "1000 changes allocated {many} bytes in total, against {once} for one"
    );
}

#[test]
fn repeated_changes_hold_one_precinct_list() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/streams/packets_poc_rpcl.j2k"
    ))
    .unwrap();

    // The tile part's POC marker, followed by an RPCL change over the 4 layers,
    // 6 resolutions and 3 components of the image, repeated as often as the
    // marker's length allows. The packets are all included by then, so the image
    // stays the same.
    let sot = data.windows(2).position(|w| w == [0xFF, 0x90]).unwrap();
    let poc = data.windows(2).position(|w| w == [0xFF, 0x5F]).unwrap();
    assert!(sot < poc);
    let len = u16::from_be_bytes([data[poc + 2], data[poc + 3]]) as usize;
    let end = poc + 2 + len;
    let last = [0, 0, 0, 4, 6, 3, 2];
    let copies = (usize::from(u16::MAX) - len) / 7;
    let mut repeated = data[..poc + 2].to_vec();
    repeated.extend(u16::try_from(len + 7 * copies).unwrap().to_be_bytes());
    repeated.extend(&data[poc + 4..end]);
    for _ in 0..copies {
        repeated.extend(last);
    }
    repeated.extend(&data[end..]);
    let psot = sot + 6..sot + 10;
    let tile_part_len = u32::from_be_bytes(data[psot.clone()].try_into().unwrap());
    let tile_part_len = tile_part_len + 7 * u32::try_from(copies).unwrap();
    repeated[psot].copy_from_slice(&tile_part_len.to_be_bytes());

    let (expected, once, _) = decode(&data);
    let (actual, many, _) = decode(&repeated);
    assert!(actual == expected, "the image changed");
    assert!(
        many < once + once / 2,
        "{copies} more changes took {many} bytes at peak, against {once}"
    );
}
