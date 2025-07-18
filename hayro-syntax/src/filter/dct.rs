use crate::object::dict::Dict;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(data: &[u8], _: Dict) -> Option<Vec<u8>> {
    // TODO: Handle the color transform attribute
    let mut decoder = zune_jpeg::JpegDecoder::new(data);
    decoder.decode_headers().ok()?;

    let mut out_colorspace = match decoder.get_input_colorspace().unwrap() {
        ColorSpace::RGB | ColorSpace::RGBA | ColorSpace::YCbCr => ColorSpace::RGB,
        ColorSpace::Luma | ColorSpace::LumaA => ColorSpace::Luma,
        ColorSpace::CMYK => ColorSpace::CMYK,
        ColorSpace::YCCK => ColorSpace::YCCK,
        _ => ColorSpace::RGB,
    };

    decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
    let mut decoded = decoder.decode().ok().or_else(|| {
        let mut decoder = zune_jpeg::JpegDecoder::new(data);
        decoder.decode_headers().ok()?;
        // It's possible that the APP14 marker is set, so that zune_jpeg will set the input colorspace
        // to a different one. So try decoding again with the different color space. This is probably
        // not the proper way to solve this, but it solves a test case.
        if matches!(out_colorspace, ColorSpace::YCCK | ColorSpace::CMYK) {
            out_colorspace = ColorSpace::RGB;
        } else {
            out_colorspace = ColorSpace::CMYK;
        }

        decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
        decoder.decode().ok()
    })?;

    if out_colorspace == ColorSpace::YCCK {
        // See <https://github.com/mozilla/pdf.js/blob/69595a29192b7704733404a42a2ebb537601117b/src/core/jpg.js#L1331>
        for c in decoded.chunks_mut(4) {
            let y = c[0] as f32;
            let cb = c[1] as f32;
            let cr = c[2] as f32;
            c[0] = (434.456 - y - 1.402 * cr) as u8;
            c[1] = (119.541 - y + 0.344 * cb + 0.714 * cr) as u8;
            c[2] = (481.816 - y - 1.772 * cb) as u8;
        }
    }

    Some(decoded)
}
