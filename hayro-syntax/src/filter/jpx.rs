use crate::bit_reader::BitWriter;
use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use hayro_jpeg2000::{ColorSpace, DecodeSettings};

impl ImageColorSpace {
    fn num_components(&self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
            Self::Unknown(num) => *num,
        }
    }
}

pub(crate) fn decode(data: &[u8], params: &ImageDecodeParams) -> Option<FilterResult<'static>> {
    use crate::object::stream::ImageColorSpace;

    let settings = DecodeSettings {
        resolve_palette_indices: false,
        strict: false,
        target_resolution: params.target_dimension,
        cancel: None,
        // With a `/ColorSpace` in the image dictionary, the colour space
        // information in the JPX data is ignored (PDF 2.0, 8.9.5.3 table 89),
        // so a raw codestream's components are not guessed to be YCbCr.
        guess_subsampled_ycbcr: params.num_components.is_none(),
    };

    let image = hayro_jpeg2000::Image::new(data, &settings).ok()?;

    let width = image.width();
    let height = image.height();
    let bpc = if params.is_indexed {
        params.bpc.unwrap_or(image.original_bit_depth())
    } else {
        8
    };
    let cs = match image.color_space() {
        ColorSpace::Gray => ImageColorSpace::Gray,
        ColorSpace::RGB => ImageColorSpace::Rgb,
        ColorSpace::CMYK => ImageColorSpace::Cmyk,
        ColorSpace::Unknown { num_channels } => ImageColorSpace::Unknown(*num_channels),
        ColorSpace::Icc {
            num_channels: num_components,
            ..
        } => match num_components {
            1 => ImageColorSpace::Gray,
            3 => ImageColorSpace::Rgb,
            4 => ImageColorSpace::Cmyk,
            _ => return None,
        },
    };
    let mut has_alpha = image.has_alpha();
    let mut cs = cs;

    // A component the decoder takes as opacity (a channel definition box, or
    // one component more than its colour space has) is a colour component
    // when the image dictionary's colour space has room for it: the colour
    // space information in the JPX data is ignored then, and `/SMaskInData`
    // images have one component fewer in their colour space.
    if has_alpha && !params.is_indexed && params.num_components == Some(cs.num_components() + 1) {
        has_alpha = false;
        cs = match cs.num_components() + 1 {
            1 => ImageColorSpace::Gray,
            3 => ImageColorSpace::Rgb,
            4 => ImageColorSpace::Cmyk,
            n => ImageColorSpace::Unknown(n),
        };
    }
    let mut decoder_context = hayro_jpeg2000::DecoderContext::default();
    let bitmap = image.decode(&mut decoder_context).ok()?.data_u8().ok()?;

    let (mut data, mut alpha) = if !has_alpha {
        (bitmap, None)
    } else {
        // Extract the alpha channel.
        let total_channels = cs.num_components() + 1;
        let mut color_channels = Vec::with_capacity(
            (bitmap.len() / total_channels as usize) * cs.num_components() as usize,
        );
        let mut alpha_channel = Vec::with_capacity(bitmap.len() / total_channels as usize);

        for sample in bitmap.chunks_exact(total_channels as usize) {
            let (alpha, color) = sample.split_last()?;
            alpha_channel.push(*alpha);
            color_channels.extend_from_slice(color);
        }

        (color_channels, Some(alpha_channel))
    };

    // The decoded image is always 8-bit, so if necessary we have to rescale
    // ourselves.
    if bpc != 8 {
        data = scale(&data, bpc, cs.num_components(), width, height)?;
        alpha = alpha.and_then(|alpha| scale(&alpha, bpc, cs.num_components(), width, height));
    }

    Some(FilterResult {
        data: Cow::Owned(data),
        image_data: Some(ImageData {
            alpha,
            color_space: Some(cs),
            bits_per_component: bpc,
            width,
            height,
        }),
    })
}

fn scale(
    data: &[u8],
    bit_per_component: u8,
    num_components: u8,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if bit_per_component == 0 || bit_per_component > 32 {
        return None;
    }

    let div_factor = ((1 << 8) - 1) as f32;
    let mul_factor = ((1_u32 << bit_per_component) - 1) as f32;

    let bits_per_row = (width as usize)
        .checked_mul(num_components as usize)?
        .checked_mul(bit_per_component as usize)?;
    let input_len = bits_per_row.div_ceil(8).checked_mul(height as usize)?;
    let mut input = vec![0; input_len];
    let mut writer = BitWriter::new(&mut input, bit_per_component)?;
    let components_per_row = (num_components as usize).checked_mul(width as usize)?;

    for bytes in data.chunks_exact(components_per_row) {
        for byte in bytes {
            let scaled = ((*byte as f32 / div_factor) * mul_factor + 0.5) as u32;
            writer.write(scaled)?;
        }

        writer.align();
    }

    let final_pos = writer.cur_pos();
    input.truncate(final_pos);

    Some(input)
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::object::stream::{ImageColorSpace, ImageDecodeParams};

    // Both from pdf.js's `jpx_subsampling.pdf`: raw codestreams of 32x32 pixels.
    // The first has three components, the last two subsampled 2x2, in an image
    // with `/ColorSpace /DeviceRGB`; the second has two, the second subsampled
    // 2x1, in an image with a two-component `/DeviceN` colour space.
    const RGB_SUBSAMPLED: &[u8] = include_bytes!("../../tests/jpx/rgb_subsampled.j2k");
    const TWO_COMPONENTS: &[u8] = include_bytes!("../../tests/jpx/two_components.j2k");

    fn params(num_components: Option<u8>) -> ImageDecodeParams {
        ImageDecodeParams {
            num_components,
            width: 32,
            height: 32,
            ..Default::default()
        }
    }

    #[test]
    fn subsampled_components_are_not_ycbcr_with_a_pdf_colour_space() {
        // The components as they are: black in the top left corner, as
        // pdf.js and MuPDF draw it.
        let decoded = decode(RGB_SUBSAMPLED, &params(Some(3))).unwrap();
        assert_eq!(decoded.data[..3], [0, 0, 0]);
        // Without a colour space in the PDF, the components are guessed to be
        // YCbCr as before.
        let guessed = decode(RGB_SUBSAMPLED, &params(None)).unwrap();
        assert_eq!(guessed.data[..3], [0, 135, 0]);
    }

    #[test]
    fn every_component_is_colour_when_the_pdf_colour_space_has_them_all() {
        let decoded = decode(TWO_COMPONENTS, &params(Some(2))).unwrap();
        let image = decoded.image_data.unwrap();
        assert!(image.alpha.is_none());
        assert!(matches!(
            image.color_space,
            Some(ImageColorSpace::Unknown(2))
        ));
        assert_eq!(decoded.data.len(), 32 * 32 * 2);
    }
}
