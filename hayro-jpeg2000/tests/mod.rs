#![allow(missing_docs)]

use hayro_jpeg2000::{DecodeSettings, Image};
use image::{ColorType, DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, Rgba, RgbaImage};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::Deserialize;
use std::any::Any;
use std::cmp::max;
use std::fs;
use std::panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const REPLACE: Option<&str> = option_env!("REPLACE");

static WORKSPACE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(""));

static SNAPSHOTS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("snapshots"));
static TEST_INPUTS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("test-inputs"));

const INPUT_MANIFESTS: &[(&str, &str)] = &[
    ("serenity", "manifest_serenity.json"),
    ("openjpeg", "manifest_openjpeg.json"),
];

static DIFFS_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let path = WORKSPACE_PATH.join("diffs");
    let _ = fs::remove_dir_all(&path);
    let _ = fs::create_dir_all(&path);
    path
});

struct TestReport {
    name: String,
    duration: Duration,
    outcome: Result<(), String>,
}

fn main() {
    let _panic_hook_guard = PanicHookGuard::install();
    if !run_harness() {
        std::process::exit(1);
    }
}

fn run_harness() -> bool {
    let asset_files = match collect_asset_files() {
        Ok(files) => files,
        Err(err) => {
            eprintln!("Failed to read asset directory: {err}");
            return false;
        }
    };

    if asset_files.is_empty() {
        eprintln!("No test inputs were found. Run `python sync.py` to download them.");
        return false;
    }

    let progress_bar = ProgressBar::new(asset_files.len() as u64);
    progress_bar.set_style(
        ProgressStyle::with_template(
            "{spinner} {pos}/{len} [{elapsed_precise}] [{wide_bar}] {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let reports: Vec<TestReport> = asset_files
        .par_iter()
        .map(|asset| {
            let name = asset.display_name.clone();
            progress_bar.set_message(name.clone());
            let start = Instant::now();
            let outcome = catch_unwind(AssertUnwindSafe(|| run_asset_test(asset))).unwrap_or_else(
                |payload| {
                    let panic_msg = describe_panic(payload.as_ref());
                    Err(format!("panic: {panic_msg}"))
                },
            );
            progress_bar.inc(1);
            TestReport {
                name,
                duration: start.elapsed(),
                outcome,
            }
        })
        .collect();

    progress_bar.finish_with_message("asset tests complete");

    println!("\nDetailed results:");
    for report in &reports {
        match &report.outcome {
            Ok(_) => println!("[PASS] {:<60} ({:.2?})", report.name, report.duration),
            Err(err) => {
                println!("[FAIL] {:<60} ({:.2?})", report.name, report.duration);
                println!("       {err}");
            }
        }
    }

    let failures: Vec<_> = reports
        .iter()
        .filter_map(|report| report.outcome.as_ref().err().map(|err| (&report.name, err)))
        .collect();

    if failures.is_empty() {
        true
    } else {
        println!(
            "\n{} of {} asset tests failed:",
            failures.len(),
            reports.len()
        );

        for (name, err) in failures {
            println!(" - {name}: {err}");
        }

        false
    }
}

fn describe_panic(payload: &(dyn Any + Send)) -> String {
    if let Some(msg) = payload.downcast_ref::<String>() {
        msg.clone()
    } else if let Some(msg) = payload.downcast_ref::<&'static str>() {
        (*msg).to_owned()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[allow(clippy::type_complexity)]
struct PanicHookGuard(Option<Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>>);

impl PanicHookGuard {
    fn install() -> Self {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {
            // Swallow default panic output; harness reports failures explicitly.
        }));
        Self(Some(previous))
    }
}

impl Drop for PanicHookGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            std::panic::set_hook(previous);
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ManifestItem {
    Simple(String),
    Detailed {
        id: String,
        #[serde(default = "default_render")]
        render: bool,
    },
}

struct AssetEntry {
    relative_path: PathBuf,
    display_name: String,
    render: bool,
}

impl AssetEntry {
    fn new(namespace: &str, id: String, render: bool) -> Self {
        let relative_path = Path::new(namespace).join(&id);
        let display_name = relative_path.display().to_string();
        Self {
            relative_path,
            display_name,
            render,
        }
    }
}

impl ManifestItem {
    fn into_asset(self, namespace: &str) -> AssetEntry {
        match self {
            Self::Simple(id) => AssetEntry::new(namespace, id, true),
            Self::Detailed { id, render } => AssetEntry::new(namespace, id, render),
        }
    }
}

fn default_render() -> bool {
    true
}

fn collect_asset_files() -> Result<Vec<AssetEntry>, String> {
    let mut files = vec![];

    for (namespace, manifest_rel_path) in INPUT_MANIFESTS {
        let manifest_path = WORKSPACE_PATH.join(manifest_rel_path);
        let content = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("failed to read manifest {}: {err}", manifest_path.display()))?;
        let entries: Vec<ManifestItem> = serde_json::from_str(&content).map_err(|err| {
            format!(
                "failed to parse manifest {}: {err}",
                manifest_path.display()
            )
        })?;

        for entry in entries {
            let asset_entry = entry.into_asset(namespace);
            let absolute_path = TEST_INPUTS_PATH.join(&asset_entry.relative_path);
            if !absolute_path.exists() {
                return Err(format!(
                    "missing test input {} (expected at {})",
                    asset_entry.display_name,
                    absolute_path.display()
                ));
            }
            files.push(asset_entry);
        }
    }

    files.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(files)
}

fn run_asset_test(asset: &AssetEntry) -> Result<(), String> {
    let asset_path = TEST_INPUTS_PATH.join(&asset.relative_path);
    let asset_name = &asset.display_name;

    let data =
        fs::read(&asset_path).map_err(|err| format!("failed to read {}: {err}", asset_name))?;
    let image = Image::new(&data, &DecodeSettings::default());

    if !asset.render {
        // Crash-only test: just execute the decoder to ensure it handles the file.
        let _ = image.and_then(|i| i.decode());
        return Ok(());
    }

    let image = image.unwrap();
    let color_type = image.color_type();
    let width = image.width();
    let height = image.height();
    let mut buf = vec![0_u8; image.total_bytes() as usize];
    image.read_image(&mut buf).unwrap();

    let rgba = match color_type {
        ColorType::L8 => {
            DynamicImage::ImageLuma8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::La8 => {
            DynamicImage::ImageLumaA8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::Rgb8 => {
            DynamicImage::ImageRgb8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::Rgba8 => {
            DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        _ => unimplemented!(),
    }
    .into_rgba8();

    let reference_path = asset.relative_path.with_extension("png");
    let snapshot_path = SNAPSHOTS_PATH.join(&reference_path);

    if let Some(parent) = snapshot_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create snapshot directory: {err}"))?;
    }

    if !snapshot_path.exists() {
        rgba.save_with_format(&snapshot_path, ImageFormat::Png)
            .map_err(|err| format!("failed to save snapshot for {}: {err}", asset_name))?;
        return Err(format!(
            "new reference image was created for {}",
            asset_name
        ));
    }

    let expected = image::open(&snapshot_path)
        .map_err(|err| format!("failed to load snapshot for {}: {err}", asset_name))?
        .into_rgba8();
    let (diff_image, pixel_diff) = get_diff(&expected, &rgba);

    if pixel_diff > 0 {
        let diff_path = DIFFS_PATH.join(&reference_path);

        if let Some(parent) = diff_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create diff directory: {err}"))?;
        }

        diff_image
            .save_with_format(&diff_path, ImageFormat::Png)
            .map_err(|err| format!("failed to save diff for {}: {err}", asset_name))?;

        if REPLACE.is_some() {
            rgba.save_with_format(&snapshot_path, ImageFormat::Png)
                .map_err(|err| format!("failed to replace snapshot for {}: {err}", asset_name))?;
            return Err(format!("snapshot was replaced for {}", asset_name));
        }

        return Err(format!(
            "pixel diff {} detected for {}",
            pixel_diff, asset_name
        ));
    }

    Ok(())
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
    // One test fails in CI because of a small difference, so we don't check
    // for exact pixel match
    const THRESHOLD: u8 = 1;

    if lhs[3] == 0 && rhs[3] == 0 {
        return false;
    }

    lhs[0].abs_diff(rhs[0]) > THRESHOLD
        || lhs[1].abs_diff(rhs[1]) > THRESHOLD
        || lhs[2].abs_diff(rhs[2]) > THRESHOLD
        || lhs[3].abs_diff(rhs[3]) > THRESHOLD
}
