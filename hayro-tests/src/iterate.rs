use hayro::Pdf;
use memchr::memmem::Finder;
use rayon::prelude::*;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use walkdir::WalkDir;

#[rustfmt::skip]
static IGNORE_LIST: &[&str] = &[
    // Password-protected
    "0000300", "0004569", "0006766", "0007159", "0008404", "0010697", "0015407", "0021311", 
    "0024617", "0023496", "0025709", "0023957", "0032605", "0017669", "0030672", "0018317", 
    "0029028", "0029047", "0031090", "0029063", "0023040",
    
    // Broken but works in other viewers
    "0010055", "0012156", "0026666",
    
    // Broken PDF, maybe fixable
    "0000399", "0003304", "0016072", "0017877", "0027069", "0027591", 
    
    // HTML
    "0000819", "0000920", "0001589", "0002064", "0002187", "0002244", "0002372", "0002554",
    "0002638", "0002966", "0003269", "0003892", "0003927", "0003983", "0004537", "0004889",
    "0004997", "0006169", "0006207", "0006339", "0006844", "0008443", "0008674", "0008978",
    "0009309", "0009464", "0009706", "0010117", "0010216", "0010902", "0011171", "0011398", 
    "0012117", "0012730", "0013178", "0013425", "0013587", "0013721", "0014006", "0014380", 
    "0015073", "0015740", "0016112", "0016335", "0016620", "0027676", "0027711", "0027958", 
    "0030263", "0028017", "0026590", "0022312", "0026641", "0029294", "0026660", "0018066", 
    "0026686", "0028171", "0026831", "0030557", "0021599", "0022634", "0018346", "0028367", 
    "0029696", "0018417", "0021747", "0021857", "0022077", "0030922", "0031948", "0027492", 
    "0024274", "0019413", "0032055", "0019706", "0032086", "0032096", "0019762", "0019782", 
    "0031403", "0032231", "0019637", "0020498", "0032430", "0021142", "0032755", "0031769", 
    "0021090", "0020881", "0032878", "0025439", "0033538", "0029331",
    
    // Invalid PDFs
    "0002229", "0002883", "0002897", "0003147", "0004099", "0004791", "0004853", "0005482", 
    "0005637", "0006036", "0006262", "0007559", "0009290", "0009944", "0010114", "0010472",
    "0010802", "0010950", "0011041", "0011758", "0011989", "0012684", "0013051", "0013338", 
    "0013822", "0014523", "0016181", "0016883"
];

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
        let data = Arc::new(fs::read(path).unwrap());
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();

        if IGNORE_LIST.contains(&name.as_str()) {
            return;
        }

        match Pdf::new(data.clone()) {
            Ok(_) => {}
            Err(e) => {
                let html_finder = Finder::new("html");
                let script_finder = Finder::new("<script");
                let reason = if html_finder.find(data.as_slice()).is_some()
                    || script_finder.find(data.as_slice()).is_some()
                {
                    "html".to_string()
                } else {
                    format!("{:?}", e)
                };
                println!("{} - {}", name, reason);
            }
        }

        let count = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count.is_multiple_of(2000) {
            println!("Processed {} PDFs", count);
        }
    });
}
