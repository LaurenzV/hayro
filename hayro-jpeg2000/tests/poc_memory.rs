//! Memory of overlapping progression order changes (POC, A.6.6), in a binary of
//! its own as it counts every allocation.

use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The decoded samples and the peak of memory allocated while decoding.
fn decode(data: &[u8]) -> (Vec<u8>, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let image = Image::new(data, &DecodeSettings::default()).unwrap();
    let mut ctx = DecoderContext::default();
    let samples = image.decode(&mut ctx).unwrap().data_u8();
    (samples, PEAK.load(Ordering::Relaxed) - base)
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

    let (expected, once) = decode(&data);
    let (actual, many) = decode(&repeated);
    assert!(actual == expected, "the image changed");
    assert!(
        many < once + once / 2,
        "{copies} more changes took {many} bytes at peak, against {once}"
    );
}
