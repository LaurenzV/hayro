use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use hayro_common::bit::BitWriter;
use hayro_jpeg2000::{ColorSpace, DecodeSettings};

impl ImageColorSpace {
    fn num_components(&self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

pub(crate) fn decode(data: &[u8], params: &ImageDecodeParams) -> Option<FilterResult> {
    use crate::object::stream::ImageColorSpace;

    let settings = DecodeSettings {
        resolve_palette_indices: false,
        strict: false,
        target_resolution: params.target_dimension,
    };

    let image = hayro_jpeg2000::Image::new(data, &settings).ok()?;

    let width = image.width();
    let height = image.height();
    let bpc = params.bpc.unwrap_or(image.original_bit_depth());
    let cs = match image.color_space() {
        ColorSpace::Gray => ImageColorSpace::Gray,
        ColorSpace::RGB => ImageColorSpace::Rgb,
        ColorSpace::CMYK => ImageColorSpace::Cmyk,
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
    let has_alpha = image.has_alpha();
    let bitmap = image.decode().ok()?;

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
        data,
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
    let div_factor = ((1 << 8) - 1) as f32;
    let mul_factor = ((1 << bit_per_component) - 1) as f32;

    let mut input = vec![
        0;
        (width as u64 * num_components as u64 * bit_per_component as u64).div_ceil(8)
            as usize
            * height as usize
    ];
    let mut writer = BitWriter::new(&mut input, bit_per_component)?;

    for bytes in data.chunks_exact(num_components as usize * width as usize) {
        for byte in bytes {
            let scaled = ((*byte as f32 / div_factor) * mul_factor).round() as u32;
            writer.write(scaled)?;
        }

        writer.align();
    }

    let final_pos = writer.cur_pos();
    input.truncate(final_pos);

    Some(input)
}
