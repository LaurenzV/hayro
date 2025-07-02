use std::sync::Arc;
use hayro_render::render_png;
use hayro_syntax::pdf::Pdf;

fn render_fuzzed(file: &[u8]) {
    let data = Arc::new(file.to_vec());
    let pdf = Pdf::new(data);

    if let Some(pdf) = pdf {
        let _pixmaps = render_png(&pdf, 1.0, None);
    }
}

#[test]
fn issue_55() {
    let file = b"3 0 obj<< /eg 4>>stream
/Dc cs/endstream6 0obj<</Type/Page
/Contents 3 0 R>>

2 0obj<</Kids[ 6 0 R ]
>>b1 0obj<</Pages 2 0R/Size 7/Root 1 0R>>";
    
    render_fuzzed(file);
}