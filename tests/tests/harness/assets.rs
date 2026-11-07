use crate::WORKSPACE_PATH;
use crate::harness::runner::{Outcome, Test};
use crate::harness::ui::Ui;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const BASE_URL: &str = "https://hayro-assets.dev";
const DOWNLOAD_THREADS: usize = 8;

pub(crate) const FALLBACK_FONTS: [&str; 6] = [
    "assets/NotoSansCJKjp-Bold.otf",
    "assets/NotoSansCJKjp-Regular.otf",
    "assets/NotoSansCJKkr-Bold.otf",
    "assets/NotoSansCJKkr-Regular.otf",
    "assets/NotoSansCJKsc-Bold.otf",
    "assets/NotoSansCJKsc-Regular.otf",
];

/// Downloads the files the given tests need that aren't present yet. Returns the outcome of
/// every test that can't run because one of its files is still missing.
pub(crate) fn fetch(tests: &[Test], offline: bool, ui: &mut Ui) -> HashMap<String, Outcome> {
    let missing: BTreeSet<&str> = tests
        .iter()
        .flat_map(|test| test.files.iter().map(String::as_str))
        .filter(|file| !WORKSPACE_PATH.join(file).exists())
        .collect();

    let mut errors: HashMap<&str, Outcome> = HashMap::new();
    let mut downloads = vec![];
    for file in missing {
        match url(file) {
            Some(_) if offline => {
                errors.insert(
                    file,
                    Outcome::Skipped("files not downloaded (offline)".into()),
                );
            }
            Some(url) => downloads.push((file, url)),
            None => {
                errors.insert(file, Outcome::Failed(format!("missing file {file}")));
            }
        }
    }

    if !downloads.is_empty() {
        for (file, error) in download(&downloads, ui) {
            errors.insert(
                file,
                Outcome::Failed(format!("couldn't download {file}: {error}")),
            );
        }
    }

    tests
        .iter()
        .filter_map(|test| {
            let outcome = test
                .files
                .iter()
                .find_map(|file| errors.get(file.as_str()))?;
            Some((test.name.clone(), outcome.clone()))
        })
        .collect()
}

fn url(file: &str) -> Option<String> {
    if let Some(path) = file.strip_prefix("downloads/") {
        Some(format!("{BASE_URL}/{path}"))
    } else if FALLBACK_FONTS.contains(&file) {
        let name = file.strip_prefix("assets/")?;
        Some(format!("{BASE_URL}/fonts/{name}"))
    } else {
        None
    }
}

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
});

pub(crate) fn download_reference(hash: &str, file: &Path) -> Result<u64, String> {
    let url = format!("{BASE_URL}/references/{hash}.png");
    download_one(&AGENT, &url, &file.to_string_lossy())
}

fn download<'a>(downloads: &[(&'a str, String)], ui: &mut Ui) -> Vec<(&'a str, String)> {
    let agent = &*AGENT;

    let next = AtomicUsize::new(0);
    let mut failures = vec![];
    let mut bytes = 0;
    let mut downloaded = 0;

    ui.download_started(downloads.len());

    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();

        for _ in 0..DOWNLOAD_THREADS.min(downloads.len()) {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                while let Some((file, url)) = downloads.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let _ = sender.send((*file, download_one(agent, url, file)));
                }
            });
        }
        drop(sender);

        for (file, result) in receiver {
            match result {
                Ok(size) => {
                    bytes += size;
                    downloaded += 1;
                    ui.download_finished(file, None);
                }
                Err(error) => {
                    ui.download_finished(file, Some(&error));
                    failures.push((file, error));
                }
            }
        }
    });

    ui.download_done(downloaded, bytes);

    failures
}

fn download_one(agent: &ureq::Agent, url: &str, file: &str) -> Result<u64, String> {
    let destination = WORKSPACE_PATH.join(file);
    let partial = WORKSPACE_PATH.join(format!("{file}.part"));

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut reader = response.into_body().into_reader();
    let mut output = std::fs::File::create(&partial).map_err(|e| e.to_string())?;

    let result = std::io::copy(&mut reader, &mut output)
        .map_err(|e| e.to_string())
        .and_then(|size| {
            std::fs::rename(&partial, &destination).map_err(|e| e.to_string())?;
            Ok(size)
        });

    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }

    result
}
