use hayro::Pdf;
use hayro_jpeg2000::DecodeSettings;
use hayro_syntax::Filter;
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

fn load_list(name: &str) -> std::io::Result<HashSet<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(format!("{}.txt", name));
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

fn load_corpus_ignore_list() -> std::io::Result<HashSet<String>> {
    load_list("corpus_ignore_list")
}

fn load_jpx_list() -> std::io::Result<HashSet<String>> {
    load_list("jpx_images")
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <folder>", args[0]);
        std::process::exit(1);
    }

    let folder = &args[1];
    check_jpx_images(folder);
}

fn load_pdf_paths(folder: &str, mut custom_condition: impl FnMut(&str) -> bool) -> Vec<PathBuf> {
    let mut pdf_paths: Vec<PathBuf> = WalkDir::new(folder)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();

            path.extension()
                .unwrap_or_default()
                .eq_ignore_ascii_case("pdf")
                && !IGNORE_LIST.contains(&name)
                && custom_condition(&name)
        })
        .collect();

    pdf_paths.sort();

    pdf_paths
}

fn check_jpx_images(folder: &str) {
    let jpx_list = load_jpx_list().unwrap();
    let paths = load_pdf_paths(folder, |name| jpx_list.contains(name));

    println!("Found {} PDF files with JPX images", paths.len());

    let count = AtomicU32::new(0);

    paths.par_iter().for_each(|path| {
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        let data = Arc::new(fs::read(path).unwrap());

        match Pdf::new(data.clone()) {
            Ok(pdf) => {
                for object in pdf.objects() {
                    if let Some(stream) = object.into_stream()
                        && stream.filters().first() == Some(&Filter::JpxDecode)
                    {
                        let raw_data = stream.raw_data();

                        match hayro_jpeg2000::read(raw_data.as_ref(), &DecodeSettings::default()) {
                            Ok(_) => {
                                // println!("ok!")
                            }
                            Err(e) => {
                                eprintln!("{}", name);
                                eprintln!("{}", e);
                            }
                        }
                    }
                }
            }
            Err(_) => unimplemented!(),
        }

        let count = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if count.is_multiple_of(100) {
            eprintln!("Processed {} PDFs", count);
        }
    });
}
