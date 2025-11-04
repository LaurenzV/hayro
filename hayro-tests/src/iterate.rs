use hayro::Pdf;
use memchr::memmem::Finder;
use rayon::prelude::*;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, LazyLock};
use walkdir::WalkDir;

static IGNORE_LIST: LazyLock<HashSet<String>> = LazyLock::new(|| {
    load_corpus_ignore_list().unwrap_or_else(|err| {
        panic!("failed to load ignore list: {err}");
    })
});

fn load_corpus_ignore_list() -> std::io::Result<HashSet<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("corpus_ignore_list.txt");
    let data = fs::read_to_string(&path)?;

    let mut set = HashSet::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        for entry in line.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            set.insert(entry.to_string());
        }
    }

    Ok(set)
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <folder>", args[0]);
        std::process::exit(1);
    }

    let folder = &args[1];

    let mut pdf_paths: Vec<PathBuf> = WalkDir::new(folder)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            path.extension()
                .unwrap_or_default()
                .eq_ignore_ascii_case("pdf")
        })
        .collect();

    pdf_paths.sort();

    println!("Found {} PDF files", pdf_paths.len());

    let count = AtomicU32::new(0);

    pdf_paths.par_iter().for_each(|path| {
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        if IGNORE_LIST.contains(name.as_str()) {
            return;
        }

        let data = Arc::new(fs::read(path).unwrap());

        match Pdf::new(data.clone()) {
            Ok(_) => {}
            Err(_) => {
                let html_finder = Finder::new("html");
                let script_finder = Finder::new("<script");
                let reason = if html_finder.find(data.as_slice()).is_some()
                    || script_finder.find(data.as_slice()).is_some()
                {
                    "html".to_string()
                } else {
                    "other".to_string()
                };
                println!("{} - {}", name, reason);
            }
        }

        let count = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if count.is_multiple_of(10000) {
            println!("Processed {} PDFs", count);
        }
    });
}
