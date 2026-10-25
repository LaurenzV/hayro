use crate::WORKSPACE_PATH;
use crate::harness::args::args;
use crate::harness::assets;
use crate::harness::snapshot::encode_png;
use image::{RgbaImage, load_from_memory};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

static LOCKFILE: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    let Ok(content) = std::fs::read_to_string(lockfile_path()) else {
        return BTreeMap::new();
    };

    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (key, hash) = line
                .split_once(' ')
                .unwrap_or_else(|| panic!("invalid line in references.lock: {line}"));
            (key.to_string(), hash.to_string())
        })
        .collect()
});

static RENDERED: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

// Concurrent downloads of the same reference image would write to the same file.
static DOWNLOAD: Mutex<()> = Mutex::new(());

pub(crate) fn lockfile_path() -> PathBuf {
    WORKSPACE_PATH.join("references.lock")
}

fn cache_path(hash: &str) -> PathBuf {
    WORKSPACE_PATH
        .join("references")
        .join(format!("{hash}.png"))
}

pub(crate) fn expected(key: &str) -> Option<&'static str> {
    LOCKFILE.get(key).map(String::as_str)
}

pub(crate) fn hash(image: &RgbaImage) -> String {
    let mut hasher = Sha256::new();
    hasher.update(image.width().to_le_bytes());
    hasher.update(image.height().to_le_bytes());
    hasher.update(image.as_raw());

    hasher.finalize()[..16]
        .iter()
        .fold(String::new(), |mut out, byte| {
            write!(out, "{byte:02x}").unwrap();
            out
        })
}

pub(crate) fn record(key: &str, hash: &str, image: &RgbaImage) {
    let Some(dir) = &args().ci else { return };

    RENDERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_string(), hash.to_string());

    if expected(key) != Some(hash) {
        let path = dir.join("images").join(format!("{hash}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, encode_png(image)).unwrap();
    }
}

pub(crate) fn load(hash: &str) -> Result<(RgbaImage, PathBuf), String> {
    let path = cache_path(hash);

    if !path.exists() {
        if args().offline {
            return Err(format!("reference image {hash} isn't downloaded (offline)"));
        }

        let _guard = DOWNLOAD.lock().unwrap_or_else(PoisonError::into_inner);
        if !path.exists() {
            let relative = path.strip_prefix(&*WORKSPACE_PATH).unwrap();
            assets::download_reference(hash, relative)
                .map_err(|err| format!("couldn't download reference image {hash}: {err}"))?;
        }
    }

    let data = std::fs::read(&path).map_err(|err| err.to_string())?;
    let image = load_from_memory(&data)
        .map_err(|err| format!("invalid reference image {hash}: {err}"))?
        .into_rgba8();

    Ok((image, path))
}

pub(crate) fn write_lockfile(dir: &Path) {
    let rendered = RENDERED.lock().unwrap_or_else(PoisonError::into_inner);
    let content = rendered.iter().fold(String::new(), |mut out, (key, hash)| {
        writeln!(out, "{key} {hash}").unwrap();
        out
    });
    std::fs::write(dir.join("references.lock"), content).unwrap();
}
