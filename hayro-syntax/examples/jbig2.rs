use hayro_syntax::filter::jbig2::{Chunk, Jbig2Image};

fn main() {
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
    
    image.parse_chunks(&chunks).unwrap();
}