use hayro_syntax::Pdf;
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

const LIMIT: usize = 200;
const ROOTS: &[&str] = &["downloads", "pdfs/custom"];

struct BenchResult {
    path: PathBuf,
    duration: Duration,
    page_count: usize,
    op_count: Option<usize>,
}

impl BenchResult {
    fn bench_full(path: &Path) -> Result<Self, String> {
        let data = fs::read(path).map_err(|err| format!("read failed: {err}"))?;

        let start = Instant::now();
        let pdf = Pdf::new(data).map_err(|err| format!("load failed: {err:?}"))?;
        let pages = pdf.pages();

        let mut op_count = 0;
        for page in pages.iter() {
            let mut iter = page.typed_operations();
            while iter.next().is_some() {
                op_count += 1;
            }
        }

        Ok(Self {
            path: path.to_path_buf(),
            duration: start.elapsed(),
            page_count: pages.len(),
            op_count: Some(op_count),
        })
    }

    fn bench_open_only(path: &Path) -> Result<Self, String> {
        let data = fs::read(path).map_err(|err| format!("read failed: {err}"))?;

        let start = Instant::now();
        let pdf = Pdf::new(data).map_err(|err| format!("load failed: {err:?}"))?;
        let duration = start.elapsed();

        Ok(Self {
            path: path.to_path_buf(),
            duration,
            page_count: pdf.pages().len(),
            op_count: None,
        })
    }
}

fn main() {
    let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = pdf_files(&base_dir);
    run_bench(
        &base_dir,
        &files,
        "Hayro syntax full",
        BenchResult::bench_full,
    );
    println!();
    run_bench(
        &base_dir,
        &files,
        "Hayro syntax open only",
        BenchResult::bench_open_only,
    );
}

fn pdf_files(base_dir: &Path) -> Vec<PathBuf> {
    let mut files = vec![];

    for root in ROOTS {
        let root = base_dir.join(root);
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if entry.file_type().is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                files.push(path.to_path_buf());
            }
        }
    }

    files.sort();
    files
}

fn run_bench(
    base_dir: &Path,
    files: &[PathBuf],
    label: &str,
    bench: impl Fn(&Path) -> Result<BenchResult, String>,
) {
    let total = files.len();
    let mut results = vec![];
    let mut failures = vec![];

    eprintln!("{label}");

    for (idx, path) in files.iter().enumerate() {
        match bench(path) {
            Ok(result) => results.push(result),
            Err(err) => failures.push((path.clone(), err)),
        }

        let processed = idx + 1;
        if processed % 500 == 0 {
            eprintln!("Processed {processed} / {total} PDFs");
        }
    }

    results.sort_by_key(|result| Reverse(result.duration));

    for result in results.iter().take(LIMIT) {
        let relative = result
            .path
            .strip_prefix(base_dir)
            .unwrap_or(result.path.as_path());

        match result.op_count {
            Some(op_count) => println!(
                "{:>10.3} ms  pages={:<4} ops={:<8} {}",
                result.duration.as_secs_f64() * 1000.0,
                result.page_count,
                op_count,
                relative.display()
            ),
            None => println!(
                "{:>10.3} ms  pages={:<4} {}",
                result.duration.as_secs_f64() * 1000.0,
                result.page_count,
                relative.display()
            ),
        }
    }

    if !failures.is_empty() {
        eprintln!("\nSkipped {} files due to errors:", failures.len());
    }
}
