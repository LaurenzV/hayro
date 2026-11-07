use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec::Vec;

/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    let globals = params
        .get::<Stream<'_>>(JBIG2_GLOBALS)
        .and_then(|g| g.decoded_data().ok());

    let settings = hayro_jbig2::DecodeSettings { strict: false };
    let globals = globals
        .map(|globals| hayro_jbig2::Globals::new(&globals, &settings))
        .transpose()
        .ok()?;
    let page = hayro_jbig2::Page::new_embedded(data, globals.as_ref(), &settings).ok()?;
    let mut ctx = hayro_jbig2::DecoderContext::default();
    let bitmap = page.decode(&mut ctx).ok()?;

    // Whenever possible (if we don't have an indexed color space), we convert
    // the data as 8-bit instead of 1-bit, so that it can be easier converted
    // into an RGBA8 image.

    // We need to invert the color because JBIG2 uses black = 1 and
    // white = 0, but PDF uses the opposite.
    let (decoded, bpc) = if image_params.is_indexed {
        let width = bitmap.width() as usize;
        let row_bytes = width.div_ceil(8);
        let mut packed = Vec::with_capacity(row_bytes * bitmap.height() as usize);
        let padding_mask = !0_u8 << ((8 - width % 8) % 8);

        for row in bitmap.rows() {
            packed.extend(bytes(row, row_bytes).map(|byte| !byte));

            if let Some(last) = packed.last_mut() {
                *last &= padding_mask;
            }
        }

        (packed, 1)
    } else {
        (bitmap.data_u8(), 8)
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: bitmap.width(),
            height: bitmap.height(),
        }),
    })
}

fn bytes(row: &[u32], len: usize) -> impl Iterator<Item = u8> + '_ {
    row.iter().flat_map(|word| word.to_be_bytes()).take(len)
}
