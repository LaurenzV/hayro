use crate::WORKSPACE_PATH;
use crate::harness::runner::{Outcome, TestResult, current_test};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

const TEMPLATE: &str = include_str!("report.html");

static PAGES: Mutex<Vec<(String, Page)>> = Mutex::new(vec![]);

pub(crate) struct Page {
    pub(crate) index: Option<usize>,
    pub(crate) reference: String,
    pub(crate) actual: String,
    pub(crate) diff: String,
    pub(crate) pixels: u32,
    pub(crate) max_delta: u8,
    pub(crate) reference_size: (u32, u32),
    pub(crate) actual_size: (u32, u32),
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
            test_pages.sort_by_key(|page| page.index);
            json!({
                "name": name,
                "message": message,
                "pages": test_pages.iter().map(|page| json!({
                    "page": page.index,
                    "reference": page.reference,
                    "actual": page.actual,
                    "diff": page.diff,
                    "pixels": page.pixels,
                    "max_delta": page.max_delta,
                    "reference_size": [page.reference_size.0, page.reference_size.1],
                    "actual_size": [page.actual_size.0, page.actual_size.1],
                })).collect::<Vec<_>>(),
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
