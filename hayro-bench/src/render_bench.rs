use hayro::hayro_interpret::InterpreterSettings;
use hayro::vello_cpu::color::palette::css::WHITE;
use pdfium_render::prelude::*;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

trait RenderBackend {
    fn name(&self) -> &'static str;
    fn render_document(
        &self,
        input_root: &Path,
        pdf_path: &Path,
        pdf_bytes: Arc<Vec<u8>>,
        save_root: Option<&Path>,
    ) -> Result<DocumentRun, String>;
}

struct PdfiumRenderBackend {
    pdfium: Pdfium,
}

struct HayroRenderBackend;

struct Cli {
    input_dir: PathBuf,
    backends: Vec<String>,
    save_bitmaps: bool,
}

struct DocumentRun {
    page_count: usize,
    total_bytes: usize,
    duration: Duration,
}

struct BackendSummary {
    name: &'static str,
    success_count: usize,
    failure_count: usize,
    total_pages: usize,
    total_bytes: usize,
    total_duration: Duration,
}

struct TableLayout {
    backend_names: Vec<&'static str>,
    name_width: usize,
    backend_widths: Vec<usize>,
    delta_width: Option<usize>,
}

enum BackendCell {
    Success(Duration),
    Failure(String),
}

const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";
const SUPPORTED_BACKENDS: [&str; 2] = ["pdfium", "hayro"];
const FAILURE_TEXT: &str = "failed to open PDF";
const TIME_EXAMPLE: &str = "12345.678 ms";
const DELTA_HEADER: &str = "hayro vs pdfium";
const DELTA_EXAMPLE: &str = "+123.45% slower";

impl PdfiumRenderBackend {
    fn new() -> Result<Self, String> {
        let bindings = Pdfium::bind_to_system_library()
            .map_err(|err| format!("failed to bind to Pdfium: {err}"))?;

        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }
}

impl HayroRenderBackend {
    fn new() -> Self {
        Self
    }
}

impl RenderBackend for PdfiumRenderBackend {
    fn name(&self) -> &'static str {
        "pdfium"
    }

    fn render_document(
        &self,
        input_root: &Path,
        pdf_path: &Path,
        pdf_bytes: Arc<Vec<u8>>,
        save_root: Option<&Path>,
    ) -> Result<DocumentRun, String> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes.as_slice(), None)
            .map_err(|err| format!("load failed: {err}"))?;
        let render_config = PdfRenderConfig::new();
        let mut total_bytes = 0usize;
        let mut page_count = 0usize;
        let mut duration = Duration::ZERO;

        let output_dir = save_root.map(|root| output_directory_for_pdf(root, input_root, pdf_path));

        if let Some(path) = output_dir.as_deref() {
            fs::create_dir_all(path).map_err(|err| {
                format!(
                    "failed to create output directory {}: {err}",
                    path.display()
                )
            })?;
        }

        for (page_index, page) in document.pages().iter().enumerate() {
            let start = Instant::now();
            let bitmap = page
                .render_with_config(&render_config)
                .map_err(|err| format!("render failed on page {}: {err}", page_index + 1))?;
            duration += start.elapsed();
            let rgba_bytes = bitmap.as_rgba_bytes();

            total_bytes += rgba_bytes.len();
            page_count += 1;

            if let Some(path) = output_dir.as_deref() {
                let file_path = path.join(format!(
                    "page_{:04}_{}x{}.rgba",
                    page_index + 1,
                    bitmap.width(),
                    bitmap.height()
                ));
                fs::write(&file_path, &rgba_bytes).map_err(|err| {
                    format!("failed to write bitmap {}: {err}", file_path.display())
                })?;
            }
        }

        Ok(DocumentRun {
            page_count,
            total_bytes,
            duration,
        })
    }
}

impl RenderBackend for HayroRenderBackend {
    fn name(&self) -> &'static str {
        "hayro"
    }

    fn render_document(
        &self,
        input_root: &Path,
        pdf_path: &Path,
        pdf_bytes: Arc<Vec<u8>>,
        save_root: Option<&Path>,
    ) -> Result<DocumentRun, String> {
        let document = hayro::hayro_syntax::Pdf::new(pdf_bytes)
            .map_err(|err| format!("load failed: {err:?}"))?;
        let interpreter_settings = InterpreterSettings::default();
        let render_settings = hayro::RenderSettings {
            bg_color: WHITE,
            ..Default::default()
        };
        let mut total_bytes = 0usize;
        let mut page_count = 0usize;
        let mut duration = Duration::ZERO;

        let output_dir = save_root.map(|root| output_directory_for_pdf(root, input_root, pdf_path));

        if let Some(path) = output_dir.as_deref() {
            fs::create_dir_all(path).map_err(|err| {
                format!(
                    "failed to create output directory {}: {err}",
                    path.display()
                )
            })?;
        }

        for (page_index, page) in document.pages().iter().enumerate() {
            let start = Instant::now();
            let pixmap = hayro::render(page, &interpreter_settings, &render_settings);
            duration += start.elapsed();
            let rgba_bytes = pixmap.data_as_u8_slice();

            total_bytes += rgba_bytes.len();
            page_count += 1;

            if let Some(path) = output_dir.as_deref() {
                let file_path = path.join(format!(
                    "page_{:04}_{}x{}.rgba",
                    page_index + 1,
                    pixmap.width(),
                    pixmap.height()
                ));
                fs::write(&file_path, rgba_bytes).map_err(|err| {
                    format!("failed to write bitmap {}: {err}", file_path.display())
                })?;
            }
        }

        Ok(DocumentRun {
            page_count,
            total_bytes,
            duration,
        })
    }
}

impl BackendSummary {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            success_count: 0,
            failure_count: 0,
            total_pages: 0,
            total_bytes: 0,
            total_duration: Duration::ZERO,
        }
    }
}

impl TableLayout {
    fn new(backends: &[Box<dyn RenderBackend>], pdfs: &[PathBuf], input_root: &Path) -> Self {
        let backend_names = backends
            .iter()
            .map(|backend| backend.name())
            .collect::<Vec<_>>();
        let mut name_width = "pdf".len();

        for pdf_path in pdfs {
            name_width = name_width.max(
                display_path(input_root, pdf_path)
                    .display()
                    .to_string()
                    .len(),
            );
        }

        let backend_widths = backend_names
            .iter()
            .map(|backend_name| {
                backend_name
                    .len()
                    .max(FAILURE_TEXT.len())
                    .max(TIME_EXAMPLE.len())
            })
            .collect::<Vec<_>>();
        let delta_width = if backend_names.contains(&"pdfium") && backend_names.contains(&"hayro") {
            Some(DELTA_HEADER.len().max(DELTA_EXAMPLE.len()))
        } else {
            None
        };

        Self {
            backend_names,
            name_width,
            backend_widths,
            delta_width,
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse()?;
    let input_dir = cli.input_dir.canonicalize().map_err(|err| {
        format!(
            "failed to access input directory {}: {err}",
            cli.input_dir.display()
        )
    })?;

    if !input_dir.is_dir() {
        return Err(format!(
            "input path is not a directory: {}",
            input_dir.display()
        ));
    }

    let backends = create_backends(&cli.backends)?;
    let pdfs = collect_pdf_files(&input_dir)?;
    if pdfs.is_empty() {
        return Err(format!("no PDF files found in {}", input_dir.display()));
    }

    let save_roots = if cli.save_bitmaps {
        let mut roots = Vec::with_capacity(backends.len());
        for backend in &backends {
            let path = derive_save_root(&input_dir, backend.name());
            fs::create_dir_all(&path)
                .map_err(|err| format!("failed to create output root {}: {err}", path.display()))?;
            roots.push((backend.name(), path));
        }
        roots
    } else {
        Vec::new()
    };

    let table_layout = TableLayout::new(&backends, &pdfs, &input_dir);
    println!(
        "backends={} pdfs={} input={}",
        table_layout.backend_names.join(","),
        pdfs.len(),
        input_dir.display()
    );
    for (backend_name, path) in &save_roots {
        println!("saving_bitmaps_{}={}", backend_name, path.display());
    }
    print_table_header(&table_layout);

    let mut summaries = backends
        .iter()
        .map(|backend| BackendSummary::new(backend.name()))
        .collect::<Vec<_>>();

    for (pdf_index, pdf_path) in pdfs.iter().enumerate() {
        let relative = display_path(&input_dir, pdf_path);
        let pdf_bytes = match fs::read(pdf_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                let error = format!("failed to open PDF ({err})");
                for summary in &mut summaries {
                    summary.failure_count += 1;
                }
                print_table_row(
                    &table_layout,
                    relative,
                    table_layout
                        .backend_names
                        .iter()
                        .map(|_| BackendCell::Failure(error.clone()))
                        .collect(),
                );
                continue;
            }
        };

        let mut cells = table_layout
            .backend_names
            .iter()
            .map(|_| BackendCell::Failure(String::from("-")))
            .collect::<Vec<_>>();
        let pdf_bytes = Arc::new(pdf_bytes);
        let execution_order = backend_execution_order(&table_layout.backend_names, pdf_index);

        for backend_name in execution_order {
            let index = table_layout
                .backend_names
                .iter()
                .position(|name| *name == backend_name)
                .unwrap();
            let backend = &backends[index];
            let save_root = save_roots
                .iter()
                .find(|(backend_name, _)| *backend_name == backend.name())
                .map(|(_, path)| path.as_path());

            match backend.render_document(&input_dir, pdf_path, Arc::clone(&pdf_bytes), save_root) {
                Ok(result) => {
                    summaries[index].success_count += 1;
                    summaries[index].total_pages += result.page_count;
                    summaries[index].total_bytes += result.total_bytes;
                    summaries[index].total_duration += result.duration;
                    cells[index] = BackendCell::Success(result.duration);
                }
                Err(err) => {
                    summaries[index].failure_count += 1;
                    cells[index] = BackendCell::Failure(err);
                }
            }
        }

        print_table_row(&table_layout, relative, cells);
    }

    println!();
    print_summary_table(&summaries);

    Ok(())
}

impl Cli {
    fn parse() -> Result<Self, String> {
        let mut args = env::args_os();
        let program = args
            .next()
            .unwrap_or_else(|| OsString::from("render_bench"));

        let mut input_dir = None;
        let mut backends = SUPPORTED_BACKENDS
            .iter()
            .map(|backend| (*backend).to_string())
            .collect::<Vec<_>>();
        let mut save_bitmaps = false;

        while let Some(arg) = args.next() {
            match arg.to_string_lossy().as_ref() {
                "--backend" => {
                    let value = args
                        .next()
                        .ok_or_else(|| String::from("--backend requires a value"))?;
                    backends = parse_backend_list(&value.to_string_lossy())?;
                }
                "--save-bitmaps" => {
                    save_bitmaps = true;
                }
                "--help" | "-h" => {
                    print_help(&program);
                    std::process::exit(0);
                }
                _ if arg.to_string_lossy().starts_with('-') => {
                    return Err(format!("unknown flag: {}", arg.to_string_lossy()));
                }
                _ => {
                    if input_dir.is_some() {
                        return Err(String::from("only one input directory may be provided"));
                    }
                    input_dir = Some(PathBuf::from(arg));
                }
            }
        }

        Ok(Self {
            input_dir: input_dir.ok_or_else(|| String::from("missing input directory"))?,
            backends,
            save_bitmaps,
        })
    }
}

fn print_help(program: &OsString) {
    println!(
        "Usage: {} <input-dir> [--backend <name>] [--save-bitmaps]",
        Path::new(program).display()
    );
    println!();
    println!("Options:");
    println!(
        "  --backend <name>     Backend to use: pdfium, hayro, comma-separated list, or all. Default: pdfium,hayro"
    );
    println!("  --save-bitmaps       Save raw RGBA page bitmaps into <input-dir>-<backend>");
}

fn create_backends(backend_names: &[String]) -> Result<Vec<Box<dyn RenderBackend>>, String> {
    let mut backends: Vec<Box<dyn RenderBackend>> = Vec::with_capacity(backend_names.len());

    for backend_name in backend_names {
        match backend_name.as_str() {
            "pdfium" => backends.push(Box::new(PdfiumRenderBackend::new()?)),
            "hayro" => backends.push(Box::new(HayroRenderBackend::new())),
            other => return Err(format!("unsupported backend: {other}")),
        }
    }

    Ok(backends)
}

fn parse_backend_list(value: &str) -> Result<Vec<String>, String> {
    if value == "all" {
        return Ok(SUPPORTED_BACKENDS
            .iter()
            .map(|backend| (*backend).to_string())
            .collect());
    }

    let mut backends = Vec::new();
    for backend in value.split(',') {
        let backend = backend.trim();
        if backend.is_empty() {
            return Err(String::from("backend list contains an empty entry"));
        }
        if !SUPPORTED_BACKENDS.contains(&backend) {
            return Err(format!("unsupported backend: {backend}"));
        }
        if !backends.iter().any(|existing| existing == backend) {
            backends.push(backend.to_string());
        }
    }

    if backends.is_empty() {
        return Err(String::from("backend list may not be empty"));
    }

    Ok(backends)
}

fn print_table_header(layout: &TableLayout) {
    print!("{:<width$}", "pdf", width = layout.name_width);
    for (backend_name, width) in layout
        .backend_names
        .iter()
        .zip(layout.backend_widths.iter())
    {
        print!("  {:>width$}", backend_name, width = *width);
    }
    if let Some(width) = layout.delta_width {
        print!("  {:>width$}", DELTA_HEADER, width = width);
    }
    println!();
}

fn print_table_row(layout: &TableLayout, pdf_name: &Path, cells: Vec<BackendCell>) {
    let mut all_success = true;
    let mut rendered_columns = Vec::with_capacity(cells.len());
    let mut pdfium_duration = None;
    let mut hayro_duration = None;

    for (backend_name, cell) in layout.backend_names.iter().copied().zip(cells) {
        match cell {
            BackendCell::Success(duration) => {
                if backend_name == "pdfium" {
                    pdfium_duration = Some(duration);
                }
                if backend_name == "hayro" {
                    hayro_duration = Some(duration);
                }
                rendered_columns.push(format!("{:.3} ms", duration.as_secs_f64() * 1000.0));
            }
            BackendCell::Failure(error) => {
                all_success = false;
                rendered_columns.push(error);
            }
        }
    }

    let color = if all_success { ANSI_GREEN } else { ANSI_RED };
    let pdf_name = pdf_name.display().to_string();

    print!("{color}{:<width$}", pdf_name, width = layout.name_width);
    for (column, width) in rendered_columns.iter().zip(layout.backend_widths.iter()) {
        print!("  {:>width$}", column, width = *width);
    }
    if let Some(width) = layout.delta_width {
        let delta = match (pdfium_duration, hayro_duration) {
            (Some(pdfium), Some(hayro)) => format_hayro_delta(pdfium, hayro),
            _ => String::from("-"),
        };
        print!("  {:>width$}", delta, width = width);
    }
    println!("{ANSI_RESET}");
}

fn format_hayro_delta(pdfium: Duration, hayro: Duration) -> String {
    let pdfium_ms = pdfium.as_secs_f64() * 1000.0;
    if pdfium_ms == 0.0 {
        return String::from("-");
    }

    let hayro_ms = hayro.as_secs_f64() * 1000.0;
    let percent = ((hayro_ms - pdfium_ms) / pdfium_ms) * 100.0;

    if percent.abs() < 0.005 {
        return String::from("0.00%");
    }
    if percent > 0.0 {
        format!("+{percent:.2}% slower")
    } else {
        format!("{:.2}% faster", -percent)
    }
}

fn backend_execution_order(backend_names: &[&'static str], pdf_index: usize) -> Vec<&'static str> {
    let mut names = backend_names.to_vec();
    if pdf_index % 2 == 1 {
        names.reverse();
    }
    names
}

fn print_summary_table(summaries: &[BackendSummary]) {
    let backend_width = summaries
        .iter()
        .map(|summary| summary.name.len())
        .max()
        .unwrap_or("backend".len())
        .max("backend".len());

    println!(
        "{:<backend_width$}  {:>4}  {:>4}  {:>5}  {:>10}  {:>10}",
        "backend",
        "ok",
        "err",
        "pages",
        "bytes",
        "total_ms",
        backend_width = backend_width
    );

    for summary in summaries {
        println!(
            "{:<backend_width$}  {:>4}  {:>4}  {:>5}  {:>10}  {:>10.3}",
            summary.name,
            summary.success_count,
            summary.failure_count,
            summary.total_pages,
            summary.total_bytes,
            summary.total_duration.as_secs_f64() * 1000.0,
            backend_width = backend_width
        );
    }
}

fn collect_pdf_files(input_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = vec![];

    for entry in WalkDir::new(input_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_type().is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

fn derive_save_root(input_dir: &Path, backend_name: &str) -> PathBuf {
    let base_name = input_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("bitmaps");

    let suffixed = format!("{base_name}-{backend_name}");

    match input_dir.parent() {
        Some(parent) => parent.join(suffixed),
        None => PathBuf::from(suffixed),
    }
}

fn output_directory_for_pdf(save_root: &Path, input_root: &Path, pdf_path: &Path) -> PathBuf {
    let relative = pdf_path
        .strip_prefix(input_root)
        .unwrap_or(pdf_path)
        .with_extension("");
    save_root.join(relative)
}

fn display_path<'a>(input_root: &'a Path, pdf_path: &'a Path) -> &'a Path {
    pdf_path.strip_prefix(input_root).unwrap_or(pdf_path)
}
