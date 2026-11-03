use crate::harness::runner::Test;
use crate::{interpreter_settings, load_pdf};
use hayro::{CmykPixmap, CmykPixmapSettings, RenderCache, RenderSettings, render_cmyk};

pub(crate) fn tests() -> Vec<Test> {
    let units: [(&str, fn()); 6] = [
        ("cmyk::ghostscript_patches", ghostscript_patches),
        ("cmyk::spot_colors", spot_colors),
        ("cmyk::blend_modes", blend_modes),
        ("cmyk::shadings", shadings),
        ("cmyk::images", images),
        ("cmyk::text_and_patterns", text_and_patterns),
    ];

    units.map(|(name, run)| Test::unit(name, run)).into()
}

fn render(path: &str) -> CmykPixmap {
    let pdf = load_pdf(path);

    render_cmyk(
        &pdf.pages()[0],
        &RenderCache::new(),
        &interpreter_settings(),
        &RenderSettings::default(),
        &CmykPixmapSettings::default(),
    )
}

/// Return the cyan, magenta, yellow, black and alpha of the pixel at `x`, `y`.
fn pixel(pixmap: &CmykPixmap, x: u16, y: u16) -> [u8; 5] {
    let start = (usize::from(y) * usize::from(pixmap.width) + usize::from(x)) * 5;
    pixmap.data[start..start + 5].try_into().unwrap()
}

/// Return the color at the center of a patch of the test files below, which hold patches
/// of 20 by 20 points in rows of 10, 10 points apart, starting 10 points from the top left.
fn patch(pixmap: &CmykPixmap, index: u16) -> [u8; 5] {
    pixel(pixmap, 20 + 30 * (index % 10), 20 + 30 * (index / 10))
}

/// Compare with Ghostscript 10.08 rendering the same file natively to CMYK
/// (`-sDEVICE=bitcmyk`), which keeps DeviceCMYK colors and blends in CMYK.
fn ghostscript_patches() {
    let pixmap = render("pdfs/custom/cmyk_patches.pdf");
    // The lower-left corners of the 60 by 60 point patches on the 180 point high page,
    // what they paint, and Ghostscript's CMYK.
    let patches = [
        (20, 100, "1 0 0 0 k", [255, 0, 0, 0]),
        (100, 100, "0 1 0 0 k", [0, 255, 0, 0]),
        (180, 100, "0 0 1 0 k", [0, 0, 255, 0]),
        (260, 100, "0 0 0 1 k", [0, 0, 0, 255]),
        (340, 100, "0 0 0 0.5 k", [0, 0, 0, 127]),
        (420, 100, "0.6 0.4 0.4 1 k", [153, 102, 102, 255]),
        (500, 100, "0.2 0.4 0.6 0.1 k", [51, 102, 153, 25]),
        (580, 100, "DeviceCMYK image 0 0 0 255", [0, 0, 0, 255]),
        (20, 20, "DeviceCMYK image 51 102 153 26", [51, 102, 153, 26]),
        (340, 20, "0.5 g", [0, 0, 0, 127]),
        (
            420,
            20,
            "0 0 0 1 k at ca 0.5 over 0 0 1 0 k",
            [0, 0, 127, 128],
        ),
        (
            500,
            20,
            "1 0 0 0 k at ca 0.5 over 0 1 0 0 k",
            [128, 127, 0, 0],
        ),
        (
            580,
            20,
            "0 0 0 1 k /Multiply over 0 0 1 0 k",
            [0, 0, 255, 255],
        ),
        // Ghostscript converts RGB through an ICC profile, so these follow the conversion
        // of the PDF specification instead.
        (100, 20, "1 0 0 rg", [0, 255, 255, 0]),
        (180, 20, "0 0 1 rg", [255, 255, 0, 0]),
        (260, 20, "1 0.5 0 rg", [0, 127, 255, 0]),
    ];

    for (x, y, paints, [c, m, y_, k]) in patches {
        let actual = pixel(&pixmap, x + 30, 180 - y - 30);
        assert_eq!(actual, [c, m, y_, k, 255], "{paints}");
    }
}

fn spot_colors() {
    let pixmap = render("pdfs/custom/cmyk_spot_colors.pdf");

    // A Separation with a DeviceCMYK alternate at tint 1 and 0.5.
    assert_eq!(patch(&pixmap, 0), [0, 127, 255, 0, 255]);
    assert_eq!(patch(&pixmap, 1), [0, 64, 127, 0, 255]);
    // A Separation with a DeviceRGB alternate.
    assert_eq!(patch(&pixmap, 2), [0, 255, 255, 0, 255]);
    // A DeviceN of cyan and black with a DeviceCMYK alternate.
    assert_eq!(patch(&pixmap, 3), [255, 0, 0, 127, 255]);
    // An Indexed color space with a DeviceCMYK base.
    assert_eq!(patch(&pixmap, 4), [0, 255, 128, 0, 255]);
    // The None separation.
    assert_eq!(patch(&pixmap, 5), [0, 0, 0, 0, 0]);
}

/// In CMYK, the separable blend modes act on the complements of the components, and for
/// the non-separable ones, black is the backdrop's for Hue, Saturation and Color, and the
/// source's for Luminosity.
fn blend_modes() {
    let pixmap = render("pdfs/custom/cmyk_blend_modes.pdf");
    // No black over full black, and half black over half black.
    let blacks = [
        ("Hue", 255, 127),
        ("Saturation", 255, 127),
        ("Color", 255, 127),
        ("Luminosity", 0, 127),
        ("Normal", 0, 127),
        ("Multiply", 255, 191),
        ("Screen", 0, 63),
    ];

    for (index, (mode, none_over_full, half_over_half)) in (0..).zip(blacks) {
        assert_eq!(
            patch(&pixmap, index),
            [0, 0, 0, none_over_full, 255],
            "{mode}"
        );
        assert_eq!(
            patch(&pixmap, index + 7),
            [0, 0, 0, half_over_half, 255],
            "{mode}"
        );
    }
}

fn shadings() {
    let pixmap = render("pdfs/custom/cmyk_shadings.pdf");

    // Both shadings run from no ink to full cyan or black across the 300 points of the
    // page, and keep the other inks at zero.
    for x in [0, 75, 150, 225, 299] {
        let expected = ((f32::from(x) + 0.5) / 300.0 * 255.0).round() as u8;
        let [c, m, y, k, a] = pixel(&pixmap, x, 10);
        assert!(c.abs_diff(expected) <= 1 && [m, y, k, a] == [0, 0, 0, 255]);
        let [c, m, y, k, a] = pixel(&pixmap, x, 40);
        assert!(k.abs_diff(expected) <= 1 && [c, m, y, a] == [0, 0, 0, 255]);
    }
}

fn images() {
    let pixmap = render("pdfs/custom/cmyk_images.pdf");

    // Adobe CMYK JPEGs of black only and of 51 102 153 26.
    assert_eq!(patch(&pixmap, 0), [0, 0, 0, 255, 255]);
    assert_eq!(patch(&pixmap, 1), [51, 102, 153, 26, 255]);
    // A black-only DeviceCMYK image with a soft mask of 50 %.
    assert_eq!(patch(&pixmap, 2), [0, 0, 0, 128, 128]);
    // A DeviceGray image of 64 becomes black only.
    assert_eq!(patch(&pixmap, 3), [0, 0, 0, 191, 255]);
    // A stencil mask painted with 0 1 1 0 k.
    assert_eq!(patch(&pixmap, 4), [0, 255, 255, 0, 255]);
}

fn text_and_patterns() {
    let pixmap = render("pdfs/custom/cmyk_text_and_patterns.pdf");
    let rows = |rows: std::ops::Range<u16>, columns: std::ops::Range<u16>| {
        rows.flat_map(move |y| columns.clone().map(move |x| (x, y)))
    };

    // Text in 0 0 0 1 k uses black ink only.
    let text = rows(0..30, 0..300).map(|(x, y)| pixel(&pixmap, x, y));
    assert!(text.clone().all(|[c, m, y, _, _]| [c, m, y] == [0, 0, 0]));
    assert!(text.clone().any(|[_, _, _, k, a]| k == 255 && a == 255));

    // A tiling pattern of black-only squares.
    let tiles = rows(40..60, 10..30).map(|(x, y)| pixel(&pixmap, x, y));
    assert!(tiles.clone().all(|[c, m, y, _, _]| [c, m, y] == [0, 0, 0]));
    assert!(tiles.clone().any(|cmyka| cmyka == [0, 0, 0, 255, 255]));
    assert!(tiles.clone().any(|cmyka| cmyka == [0, 0, 0, 0, 0]));

    // Full black under a luminosity soft mask of 50 % gray.
    assert_eq!(pixel(&pixmap, 50, 50), [0, 0, 0, 128, 128]);
}
