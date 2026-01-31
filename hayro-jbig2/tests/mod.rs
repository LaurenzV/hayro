//! Test suite for hayro-jbig2.
//!
//! Run `python sync.py` to download test assets before running tests.

use std::any::Any;
use std::cmp::max;
use std::fs;
use std::panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use image::{GrayImage, ImageFormat, Rgba, RgbaImage};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::Deserialize;

const REPLACE: Option<&str> = option_env!("REPLACE");

static WORKSPACE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

static SNAPSHOTS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("snapshots"));
static TEST_INPUTS_PATH: LazyLock<PathBuf> = LazyLock::new(|| WORKSPACE_PATH.join("test-inputs"));

static DIFFS_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let path = WORKSPACE_PATH.join("diffs");
    let _ = fs::remove_dir_all(&path);
    let _ = fs::create_dir_all(&path);
    path
});

const INPUT_MANIFESTS: &[(&str, &str)] = &[
    ("serenity", "manifest_serenity.json"),
    ("power_jbig2", "manifest_power_jbig2.json"),
];

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ManifestEntry {
    Simple(String),
    Complex {
        id: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default = "default_render")]
        render: bool,
        #[serde(default)]
        ignore: bool,
    },
}

fn default_render() -> bool {
    true
}

impl ManifestEntry {
    fn id(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Complex { id, .. } => id,
        }
    }

    fn path(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Complex { path, id, .. } => path.as_deref().unwrap_or(id),
        }
    }

    fn render(&self) -> bool {
        match self {
            Self::Simple(_) => true,
            Self::Complex { render, .. } => *render,
        }
    }

    fn ignore(&self) -> bool {
        match self {
            Self::Simple(_) => false,
            Self::Complex { ignore, .. } => *ignore,
        }
    }
}

struct AssetEntry {
    input_relative_path: PathBuf,
    snapshot_stem: PathBuf,
    display_name: String,
    render: bool,
}

impl AssetEntry {
    fn new(namespace: &str, id: String, path: String, render: bool) -> Self {
        let display_name = format!("{namespace}/{id}");
        let input_relative_path = Path::new(namespace).join(path);
        let snapshot_stem = Path::new(namespace).join(id);
        Self {
            input_relative_path,
            snapshot_stem,
            display_name,
            render,
        }
    }
}

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

fn collect_asset_files() -> Result<Vec<AssetEntry>, String> {
    let mut files = vec![];

    for (namespace, manifest_rel_path) in INPUT_MANIFESTS {
        let manifest_path = WORKSPACE_PATH.join(manifest_rel_path);

        if !manifest_path.exists() {
            continue;
        }

        let content = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("failed to read manifest {}: {err}", manifest_path.display()))?;
        let entries: Vec<ManifestEntry> = serde_json::from_str(&content).map_err(|err| {
            format!(
                "failed to parse manifest {}: {err}",
                manifest_path.display()
            )
        })?;

        for entry in entries {
            // Skip ignored entries.
            if entry.ignore() {
                continue;
            }

            let asset_entry = AssetEntry::new(
                namespace,
                entry.id().to_string(),
                entry.path().to_string(),
                entry.render(),
            );
            let absolute_path = TEST_INPUTS_PATH.join(&asset_entry.input_relative_path);

            if !absolute_path.exists() {
                // Skip missing files instead of failing.
                continue;
            }

            files.push(asset_entry);
        }
    }

    files.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(files)
}

fn run_asset_test(asset: &AssetEntry) -> Result<(), String> {
    let asset_path = TEST_INPUTS_PATH.join(&asset.input_relative_path);
    let asset_name = &asset.display_name;

    let data =
        fs::read(&asset_path).map_err(|err| format!("failed to read {}: {err}", asset_name))?;

    let image = hayro_jbig2::decode(&data).map_err(|err| format!("decode failed: {err}"))?;

    if !asset.render {
        // Crash-only test: just execute the decoder to ensure it handles the file.
        return Ok(());
    }

    // Convert 1-bit packed bitmap to 8-bit grayscale for comparison.
    let luma = bitmap_to_luma(&image);

    let reference_path = asset.snapshot_stem.with_extension("png");
    let snapshot_path = SNAPSHOTS_PATH.join(&reference_path);

    if let Some(parent) = snapshot_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create snapshot directory: {err}"))?;
    }

    if !snapshot_path.exists() {
        luma.save_with_format(&snapshot_path, ImageFormat::Png)
            .map_err(|err| format!("failed to save snapshot for {}: {err}", asset_name))?;
        return Err(format!(
            "new reference image was created for {}",
            asset_name
        ));
    }

    let expected = image::open(&snapshot_path)
        .map_err(|err| format!("failed to load snapshot for {}: {err}", asset_name))?
        .into_rgba8();
    let rgba = image::DynamicImage::ImageLuma8(luma).into_rgba8();
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

/// Convert a JBIG2 bitmap to a grayscale image.
///
/// In JBIG2, true = black, false = white. We convert to grayscale where:
/// - false (white in JBIG2) -> 255 (white in image)
/// - true (black in JBIG2) -> 0 (black in image)
fn bitmap_to_luma(image: &hayro_jbig2::Image) -> GrayImage {
    let width = image.width;
    let height = image.height;

    struct LumaDecoder {
        buffer: Vec<u8>,
    }

    impl hayro_jbig2::Decoder for LumaDecoder {
        fn push_pixel(&mut self, black: bool) {
            self.buffer.push(if black { 0 } else { 255 });
        }

        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            let luma = if black { 0 } else { 255 };
            self.buffer
                .extend(std::iter::repeat_n(luma, chunk_count as usize * 8));
        }

        fn next_line(&mut self) {}
    }

    let mut decoder = LumaDecoder {
        buffer: Vec::with_capacity((width * height) as usize),
    };

    image.decode(&mut decoder);

    GrayImage::from_raw(width, height, decoder.buffer).expect("buffer size mismatch")
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
    // For bi-level images, we expect exact matches (no threshold needed).
    lhs[0] != rhs[0] || lhs[1] != rhs[1] || lhs[2] != rhs[2] || lhs[3] != rhs[3]
}
