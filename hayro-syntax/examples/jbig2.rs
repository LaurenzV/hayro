use hayro_syntax::filter::jbig2::{Chunk, Jbig2Image};

fn main() {
    if let Ok(()) = log::set_logger(&LOGGER) {
        log::set_max_level(log::LevelFilter::Trace);
    }

    
    let data = std::fs::read("out.jb2").unwrap();
    let globals_data = std::fs::read("globals_data.jb2").unwrap();
    
    let mut image = Jbig2Image::new();
    
    let chunks = vec![
        Chunk {
            data: globals_data.clone(),
            start: 0,
            end: globals_data.len(),
        },
        Chunk {
            data: data.clone(),
            start: 0,
            end: data.len(),
        }
    ];
    
    let res = image.parse_chunks(&chunks).unwrap();
    for (idx, b) in res.iter().enumerate() {
        // println!("{idx}, {}", b);
    }
}

/// A simple stderr logger.
static LOGGER: SimpleLogger = SimpleLogger;
struct SimpleLogger;
impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let target = if !record.target().is_empty() {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };

            let line = record.line().unwrap_or(0);
            let args = record.args();

            match record.level() {
                log::Level::Error => eprintln!("Error (in {}:{}): {}", target, line, args),
                log::Level::Warn => eprintln!("Warning (in {}:{}): {}", target, line, args),
                log::Level::Info => eprintln!("Info (in {}:{}): {}", target, line, args),
                log::Level::Debug => eprintln!("Debug (in {}:{}): {}", target, line, args),
                log::Level::Trace => eprintln!("Trace (in {}:{}): {}", target, line, args),
            }
        }
    }

    fn flush(&self) {}
}