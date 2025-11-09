use hayro_jpeg2000::bitmap::Bitmap;
use hayro_jpeg2000::read;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba, RgbaImage};
use std::cmp::max;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const REPLACE: Option<&str> = option_env!("REPLACE");

static WORKSPACE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(""));

static ASSETS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("assets"));
static SNAPSHOTS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("snapshots"));

static DIFFS_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let path = WORKSPACE_PATH.join("diffs");
    let _ = fs::remove_dir_all(&path);
    let _ = fs::create_dir_all(&path);
    path
});

#[test]
fn kakadu_lossless_gray_u8_prog1_layers1_res6() {
    run_asset_test("kakadu-lossless-gray-u8-prog1-layers1-res6.jp2");
}

#[test]
fn kakadu_lossless_gray_alpha_u8_prog1_layers1_res6() {
    run_asset_test("kakadu-lossless-gray-alpha-u8-prog1-layers1-res6.jp2");
}

#[test]
fn kakadu_lossless_rgb_u8_prog1_layers1_res6_mct() {
    run_asset_test("kakadu-lossless-rgb-u8-prog1-layers1-res6-mct.jp2");
}

#[test]
fn kakadu_lossless_rgba_u8_prog1_layers1_res6_mct() {
    run_asset_test("kakadu-lossless-rgba-u8-prog1-layers1-res6-mct.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_tlm() {
    run_asset_test("openjpeg-lossless-rgba-u8-TLM.jp2");
}

#[test]
fn openjpeg_lossless_rgn() {
    run_asset_test("openjpeg-lossless-RGN.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_tile4x2_cblk4x16_tp3_layers3_res2() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-tile4x2-cblk4x16-tp3-layers3-res2.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog1_tile4x2_cblk4x16_tp3_layers3_res2() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog1-tile4x2-cblk4x16-tp3-layers3-res2.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog2_tile4x2_cblk4x16_tp3_layers3_res2() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog2-tile4x2-cblk4x16-tp3-layers3-res2.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog2_tile4x3_cblk4x16_tp3_layers3_res2() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog3-tile4x2-cblk4x16-tp3-layers3-res2.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog2_tile4x4_cblk4x16_tp3_layers3_res2() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog4-tile4x2-cblk4x16-tp3-layers3-res2.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_tile_part_index_overflow() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-tile-part-index-overflow.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_sop() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-SOP.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_eph() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-EPH.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_eph_sop() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-EPH-SOP.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_prog0_eph_empty_packets() {
    run_asset_test("openjpeg-lossless-rgba-u8-prog0-EPH-empty-packets.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u8_plt() {
    run_asset_test("openjpeg-lossless-rgba-u8-PLT.jp2");
}

#[test]
fn jasper_tile4x2_res5() {
    run_asset_test("jasper-tile4x2-res5.jp2");
}

#[test]
fn openjpeg_lossless_rgba_u4() {
    run_asset_test("openjpeg-lossless-rgba-u4.jp2");
}

#[test]
fn openjpeg_lossy_quantization_scalar_derived() {
    run_asset_test("openjpeg-lossy-quantization-scalar-derived.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_02_resetprob() {
    run_asset_test("jasper-rgba-u8-cbstyle-02-resetprob.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_04_termall() {
    run_asset_test("jasper-rgba-u8-cbstyle-04-termall.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_04_termall_layers() {
    run_asset_test("jasper-rgba-u8-cbstyle-04-termall-layers.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_06_resetprob_termall() {
    run_asset_test("jasper-rgba-u8-cbstyle-06-resetprob-termall.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_36_termall_segsym() {
    run_asset_test("jasper-rgba-u8-cbstyle-36-termall-segsym.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_08_vcausal() {
    run_asset_test("jasper-rgba-u8-cbstyle-08-vcausal.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_16_pterm() {
    run_asset_test("jasper-rgba-u8-cbstyle-16-pterm.jp2");
}

#[test]
fn jasper_rgba_u8_cbstyle_32_segsym() {
    run_asset_test("jasper-rgba-u8-cbstyle-32-segsym.jp2");
}

fn run_asset_test(file_name: &str) {
    let asset_path = ASSETS_PATH.join(file_name);
    let data = fs::read(&asset_path).expect("failed to read asset");
    let bitmap = read(&data).expect("failed to decode jp2 file");

    let rgba = bitmap_to_dynamic_image(bitmap).into_rgba8();
    let reference_name = Path::new(file_name)
        .with_extension("png")
        .file_name()
        .unwrap()
        .to_owned();

    let snapshot_path = SNAPSHOTS_PATH.join(&reference_name);
    let diff_path = DIFFS_PATH.join(&reference_name);

    fs::create_dir_all(&*SNAPSHOTS_PATH).expect("failed to create snapshots directory");

    if !snapshot_path.exists() {
        rgba.save_with_format(&snapshot_path, ImageFormat::Png)
            .expect("failed to save snapshot");
        panic!("new reference image was created for {}", file_name);
    }

    let expected = image::open(&snapshot_path)
        .expect("failed to load snapshot")
        .into_rgba8();
    let (diff_image, pixel_diff) = get_diff(&expected, &rgba);

    if pixel_diff > 0 {
        diff_image
            .save_with_format(&diff_path, ImageFormat::Png)
            .expect("failed to save diff");

        if REPLACE.is_some() {
            rgba.save_with_format(&snapshot_path, ImageFormat::Png)
                .expect("failed to replace snapshot");
            panic!("snapshot was replaced for {}", file_name);
        }

        panic!("pixel diff {} detected for {}", pixel_diff, file_name);
    }

    if diff_path.exists() {
        let _ = fs::remove_file(diff_path);
    }
}

fn bitmap_to_dynamic_image(bitmap: Bitmap) -> DynamicImage {
    let Bitmap { channels, metadata } = bitmap;
    let (width, height) = (metadata.width, metadata.height);

    let has_alpha = channels.iter().any(|c| c.is_alpha);
    let num_channels = channels.len();

    let channels = channels
        .into_iter()
        .map(|c| c.into_8bit())
        .collect::<Vec<_>>();

    let interleaved = if num_channels == 1 {
        channels[0].clone()
    } else {
        let mut interleaved = vec![];
        let num_samples = channels.iter().map(|c| c.len()).min().unwrap();

        for sample_idx in 0..num_samples {
            for channel in &channels {
                interleaved.push(channel[sample_idx]);
            }
        }

        interleaved
    };

    match (num_channels, has_alpha) {
        (1, false) => {
            DynamicImage::ImageLuma8(ImageBuffer::from_raw(width, height, interleaved).unwrap())
        }
        (2, true) => {
            DynamicImage::ImageLumaA8(ImageBuffer::from_raw(width, height, interleaved).unwrap())
        }
        (3, false) => {
            DynamicImage::ImageRgb8(ImageBuffer::from_raw(width, height, interleaved).unwrap())
        }
        (4, true) => {
            DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, interleaved).unwrap())
        }
        _ => unimplemented!(),
    }
}

fn get_diff(expected_image: &RgbaImage, actual_image: &RgbaImage) -> (RgbaImage, u32) {
    let width = max(expected_image.width(), actual_image.width());
    let height = max(expected_image.height(), actual_image.height());

    let mut diff_image = RgbaImage::new(width * 3, height);
    let mut pixel_diff = 0;

    for x in 0..width {
        for y in 0..height {
            let actual_pixel = get_pixel_checked(actual_image, x, y);
            let expected_pixel = get_pixel_checked(expected_image, x, y);

            match (actual_pixel, expected_pixel) {
                (Some(actual), Some(expected)) => {
                    diff_image.put_pixel(x, y, expected);
                    diff_image.put_pixel(x + width, y, diff_pixel(expected, actual));
                    diff_image.put_pixel(x + 2 * width, y, actual);

                    if is_pixel_different(expected, actual) {
                        pixel_diff += 1;
                    }
                }
                (Some(actual), None) => {
                    pixel_diff += 1;
                    diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                    diff_image.put_pixel(x + 2 * width, y, actual);
                }
                (None, Some(expected)) => {
                    pixel_diff += 1;
                    diff_image.put_pixel(x, y, expected);
                    diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                }
                (None, None) => {}
            }
        }
    }

    (diff_image, pixel_diff)
}

fn get_pixel_checked(image: &RgbaImage, x: u32, y: u32) -> Option<Rgba<u8>> {
    if x < image.width() && y < image.height() {
        Some(*image.get_pixel(x, y))
    } else {
        None
    }
}

fn diff_pixel(expected: Rgba<u8>, actual: Rgba<u8>) -> Rgba<u8> {
    if is_pixel_different(expected, actual) {
        Rgba([255, 0, 0, 255])
    } else {
        Rgba([0, 0, 0, 255])
    }
}

fn is_pixel_different(lhs: Rgba<u8>, rhs: Rgba<u8>) -> bool {
    if lhs[3] == 0 && rhs[3] == 0 {
        return false;
    }

    lhs[0] != rhs[0] || lhs[1] != rhs[1] || lhs[2] != rhs[2] || lhs[3] != rhs[3]
}
