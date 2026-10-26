//! This example shows you how you can convert a JPEG2000 image into PNG using
//! the `image` crate.

use std::env;
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let target = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test.jp2"));

    hayro_jpeg2000::integration::register_decoding_hook();
    let image = image::ImageReader::open(target)?.decode()?;
    image.save("out.png")?;
    Ok(())
}
