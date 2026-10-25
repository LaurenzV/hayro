use crate::WORKSPACE_PATH;
use crate::harness::args::args;
use crate::harness::runner::{Outcome, TestResult, current_test};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

const TEMPLATE: &str = include_str!("report.html");

static PAGES: Mutex<Vec<(String, Page)>> = Mutex::new(vec![]);

pub(crate) enum Page {
    Changed {
        index: Option<usize>,
        reference: String,
        actual: String,
        diff: String,
        pixels: u32,
        max_delta: u8,
        reference_size: (u32, u32),
        actual_size: (u32, u32),
    },
    Missing {
        index: Option<usize>,
        png: Vec<u8>,
        size: (u32, u32),
    },
}

impl Page {
    fn index(&self) -> Option<usize> {
        match self {
            Self::Changed { index, .. } | Self::Missing { index, .. } => *index,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Changed {
                index,
                reference,
                actual,
                diff,
                pixels,
                max_delta,
                reference_size,
                actual_size,
            } => json!({
                "page": index,
                "missing": false,
                "reference": reference,
                "actual": actual,
                "diff": diff,
                "pixels": pixels,
                "max_delta": max_delta,
                "reference_size": [reference_size.0, reference_size.1],
                "actual_size": [actual_size.0, actual_size.1],
            }),
            Self::Missing { index, png, size } => json!({
                "page": index,
                "missing": true,
                "actual": format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png)),
                "actual_size": [size.0, size.1],
            }),
        }
    }
}

pub(crate) fn html_path() -> PathBuf {
    WORKSPACE_PATH.join("report.html")
}

pub(crate) fn files_dir() -> PathBuf {
    WORKSPACE_PATH.join("report")
}

pub(crate) fn clear() {
    let _ = std::fs::remove_file(html_path());
    let _ = std::fs::remove_dir_all(files_dir());
}

pub(crate) fn add(page: Page) {
    if let Some(test) = current_test() {
        PAGES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((test, page));
    }
}

pub(crate) fn write(results: &[TestResult]) -> Option<PathBuf> {
    if args().update {
        return None;
    }

    let mut pages: HashMap<String, Vec<Page>> = HashMap::new();
    for (test, page) in std::mem::take(&mut *PAGES.lock().unwrap_or_else(PoisonError::into_inner)) {
        pages.entry(test).or_default().push(page);
    }

    let mut failed: Vec<_> = results
        .iter()
        .filter_map(|result| match &result.outcome {
            Outcome::Failed(message) => Some((&result.name, message)),
            _ => None,
        })
        .collect();
    if failed.is_empty() {
        return None;
    }
    failed.sort();

    let tests: Vec<Value> = failed
        .iter()
        .map(|(name, message)| {
            let mut test_pages = pages.remove(*name).unwrap_or_default();
            test_pages.sort_by_key(Page::index);
            json!({
                "name": name,
                "message": message,
                "pages": test_pages.iter().map(Page::to_json).collect::<Vec<_>>(),
            })
        })
        .collect();

    let data = json!({
        "total": results.len(),
        "failed": failed.len(),
        "tests": tests,
    });
    // Keeps the data from closing the surrounding script element.
    let data = data.to_string().replace("</", "<\\/");

    let path = html_path();
    std::fs::write(&path, TEMPLATE.replace("__REPORT_DATA__", &data)).ok()?;
    Some(path)
}
