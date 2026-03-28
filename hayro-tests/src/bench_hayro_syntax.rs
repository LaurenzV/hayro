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
    op_count: usize,
}

fn main() {
    let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = pdf_files(&base_dir);
    let total = files.len();
    let mut results = vec![];
    let mut failures = vec![];

    for (idx, path) in files.into_iter().enumerate() {
        match bench_file(&path) {
            Ok(result) => results.push(result),
            Err(err) => failures.push((path, err)),
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
            .strip_prefix(&base_dir)
            .unwrap_or(result.path.as_path());

        println!(
            "{:>10.3} ms  pages={:<4} ops={:<8} {}",
            result.duration.as_secs_f64() * 1000.0,
            result.page_count,
            result.op_count,
            relative.display()
        );
    }

    if !failures.is_empty() {
        eprintln!("\nSkipped {} files due to errors:", failures.len());
        for (path, err) in failures {
            let relative = path.strip_prefix(&base_dir).unwrap_or(path.as_path());
            eprintln!("  {}: {}", relative.display(), err);
        }
    }
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

fn bench_file(path: &Path) -> Result<BenchResult, String> {
    let data = fs::read(path).map_err(|err| format!("read failed: {err}"))?;

    let start = Instant::now();
    let pdf = Pdf::new(data).map_err(|err| format!("load failed: {err:?}"))?;
    let pages = pdf.pages();

    let mut op_count = 0;
    for page in pages.iter() {
        for _ in page.typed_operations() {
            op_count += 1;
        }
    }

    Ok(BenchResult {
        path: path.to_path_buf(),
        duration: start.elapsed(),
        page_count: pages.len(),
        op_count,
    })
}
