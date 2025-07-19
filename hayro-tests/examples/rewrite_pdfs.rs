use hayro::Pdf;
use pdf_writer::{Dict, Name, Obj};
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_dir = Path::new("pdfs_without_page_attrs");
    let output_dir = Path::new("rewritten_pdfs");

    if !input_dir.exists() {
        eprintln!(
            "Input directory '{}' does not exist. Run copy_pdfs_without_pages.py first.",
            input_dir.display()
        );
        return Ok(());
    }

    fs::create_dir_all(output_dir)?;

    // Collect all PDF files and sort them
    let mut pdf_files: Vec<_> = fs::read_dir(input_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("pdf") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Sort by filename
    pdf_files.sort_by(|a, b| {
        a.file_name()
            .unwrap()
            .to_string_lossy()
            .cmp(&b.file_name().unwrap().to_string_lossy())
    });

    for path in &pdf_files {
        let filename = path.file_name().unwrap();
        println!("Processing: {:?}", filename);

        let pdf_bytes = fs::read(&path)?;
        let data = Arc::new(pdf_bytes);

        match Pdf::new(data) {
            Ok(hayro_pdf) => {
                let page_count = hayro_pdf.pages().len();

                if page_count == 0 {
                    eprintln!("  Warning: No pages found in {:?}", filename);
                    continue;
                }

                let page_indices: Vec<usize> = (0..page_count).collect();

                let output_bytes =
                    hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, &page_indices);

                let output_path = output_dir.join(filename);
                fs::write(&output_path, output_bytes)?;

                println!("  Rewrote {} pages to {:?}", page_count, output_path);
            }
            Err(_) => {
                eprintln!("  Error parsing {:?}", filename);
            }
        }
    }

    println!("\nDone! Rewritten PDFs are in '{}'", output_dir.display());

    Ok(())
}
