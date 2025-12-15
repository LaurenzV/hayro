//! Test suite for hayro-jbig2.
//!
//! Run `python sync.py` to download test assets before running tests.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use indicatif::{ParallelProgressIterator, ProgressStyle};
use rayon::prelude::*;
use serde::Deserialize;

const SCRIPT_DIR: &str = env!("CARGO_MANIFEST_DIR");

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
}

struct TestCase {
    id: String,
    input_path: PathBuf,
    #[allow(dead_code)]
    render: bool,
}

fn load_manifest(namespace: &str) -> Vec<TestCase> {
    let manifest_path = PathBuf::from(SCRIPT_DIR).join(format!("manifest_{namespace}.json"));

    if !manifest_path.exists() {
        return vec![];
    }

    let content = std::fs::read_to_string(&manifest_path).unwrap();
    let entries: Vec<ManifestEntry> = serde_json::from_str(&content).unwrap();

    entries
        .into_iter()
        .map(|entry| {
            let input_path =
                PathBuf::from(SCRIPT_DIR).join(format!("test-inputs/{namespace}/{}", entry.path()));

            TestCase {
                id: entry.id().to_string(),
                input_path,
                render: entry.render(),
            }
        })
        .collect()
}

fn load_all_tests() -> Vec<(String, TestCase)> {
    let mut tests = Vec::new();

    for namespace in ["serenity"] {
        for case in load_manifest(namespace) {
            tests.push((format!("{namespace}/{}", case.id), case));
        }
    }

    tests
}

enum TestResult {
    Pass(Duration),
    Fail(String),
    Skip,
}

fn run_test(case: &TestCase) -> TestResult {
    if !case.input_path.exists() {
        return TestResult::Skip;
    }

    let data = match std::fs::read(&case.input_path) {
        Ok(d) => d,
        Err(e) => return TestResult::Fail(format!("Failed to read file: {e}")),
    };

    let start = Instant::now();
    match hayro_jbig2::decode(&data, None) {
        Ok(_image) => TestResult::Pass(start.elapsed()),
        Err(e) => TestResult::Fail(e.to_string()),
    }
}

fn main() {
    let tests = load_all_tests();

    if tests.is_empty() {
        println!("No test cases found. Run `python sync.py` to download test assets.");
        return;
    }

    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap();

    let results: Vec<_> = tests
        .par_iter()
        .progress_with_style(style)
        .map(|(name, case)| (name.clone(), run_test(case)))
        .collect();

    println!("\nDetailed results:");
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for (name, result) in &results {
        match result {
            TestResult::Pass(duration) => {
                println!("[PASS] {name:<60} ({duration:.2?})");
                passed += 1;
            }
            TestResult::Fail(msg) => {
                println!("[FAIL] {name:<60} ({msg})");
                failed += 1;
            }
            TestResult::Skip => {
                skipped += 1;
            }
        }
    }

    println!("\nSummary: {passed} passed, {failed} failed, {skipped} skipped");

    if failed > 0 {
        std::process::exit(1);
    }
}
