//! Page striping (7.4.8.6, 7.4.10).

use crate::{Bitmap, check, load_reference, read_stream};
use hayro_jbig2::{DecodeSettings, File, FormatError, PageBitmap};

fn rows_from_bitmap(bitmap: &PageBitmap) -> Bitmap {
    let width = bitmap.width() as usize;
    bitmap
        .rows()
        .map(|row| {
            (0..width)
                .map(|x| row[x / 32] & (0x8000_0000 >> (x % 32)) != 0)
                .collect()
        })
        .collect()
}

#[test]
fn striped() {
    check("page_striped", "truchet");
}

#[test]
fn striped_unknown_height() {
    check("page_striped_unknown_height", "truchet");
}

#[test]
fn striped_last_stripe_implicit() {
    check("page_striped_last_stripe_implicit", "truchet");
}

#[test]
fn striped_single_stripe() {
    check("page_striped_single_stripe", "truchet");
}

#[test]
fn striped_no_end_of_stripe() {
    check("page_striped_no_end_of_stripe", "truchet");
}

#[test]
fn progressive_step_by_step() {
    let data = read_stream("page_striped");
    let reference = load_reference("truchet");
    let file = File::new(
        &data,
        &DecodeSettings {
            strict: true,
            ..Default::default()
        },
    )
    .unwrap();
    let page = file.page(0).unwrap();

    let mut progressive = page.decode_progressive().unwrap();
    assert!(progressive.total_segments() > 2);
    assert_eq!(progressive.processed_segments(), 0);
    assert!(!progressive.is_done());

    let mut steps = 0;
    while progressive.step().unwrap() {
        steps += 1;
        assert!(progressive.processed_segments() <= progressive.total_segments());
        assert_eq!(progressive.bitmap().width() as usize, reference[0].len());
    }
    assert!(steps >= 2);
    assert!(progressive.is_done());
    // Subsequent step calls are idempotent once finished.
    assert!(!progressive.step().unwrap());

    let rows = rows_from_bitmap(&progressive.finish());
    assert_eq!(rows, reference);
}

#[test]
fn target_size_clips_and_pads() {
    let data = read_stream("page_striped");
    let reference = load_reference("truchet");
    let orig_w = reference[0].len() as u32;
    let orig_h = reference.len() as u32;

    let file = File::new(&data, &DecodeSettings::default()).unwrap();
    let page = file.page(0).unwrap();

    assert!(matches!(
        page.decode_with_size(0, orig_h),
        Err(hayro_jbig2::DecodeError::Format(FormatError::EmptyPage))
    ));

    // Smaller target dimensions clip decoded regions cleanly.
    let crop_w = (orig_w / 2).max(1);
    let crop_h = (orig_h / 2).max(1);
    let cropped = rows_from_bitmap(&page.decode_with_size(crop_w, crop_h).unwrap());
    assert_eq!(cropped.len(), crop_h as usize);
    for (actual, expected) in cropped.iter().zip(&reference[..crop_h as usize]) {
        assert_eq!(&actual[..], &expected[..crop_w as usize]);
    }

    // Larger target dimensions pad out-of-bounds pixels with the page default pixel.
    let pad_w = orig_w + 11;
    let pad_h = orig_h + 7;
    let padded = rows_from_bitmap(&page.decode_with_size(pad_w, pad_h).unwrap());
    assert_eq!(padded.len(), pad_h as usize);
    for (y, actual) in padded.iter().enumerate() {
        if y < orig_h as usize {
            assert_eq!(&actual[..orig_w as usize], reference[y].as_slice());
            assert!(actual[orig_w as usize..pad_w as usize].iter().all(|&p| !p));
        } else {
            assert!(actual[..pad_w as usize].iter().all(|&p| !p));
        }
    }
}
