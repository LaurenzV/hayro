use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use hayro_common::bit::{BitSize, BitWriter};
use jpeg2k::DecodeParameters;

impl ImageColorSpace {
    fn num_components(&self) -> u8 {
        match self {
            ImageColorSpace::Gray => 1,
            ImageColorSpace::Rgb => 3,
            ImageColorSpace::Cmyk => 4,
        }
    }
}

pub(crate) fn decode(data: &[u8], params: &ImageDecodeParams) -> Option<FilterResult> {
    use crate::object::stream::ImageColorSpace;

    let mut jpx_params = DecodeParameters::new();

    if params.is_indexed {
        jpx_params = jpx_params.ignore_pclr_cmap_cdef();
    }

    let image = jpeg2k::Image::from_bytes_with(data, jpx_params).ok()?;
    let width = image.width();
    let height = image.height();
    let components = image.components();
    let bpc = params.bpc.unwrap_or(
        components
            .iter()
            .fold(u32::MIN, |max, c| max.max(c.precision())) as u8,
    );
    let cs = match components.iter().filter(|c| !c.is_alpha()).count() {
        1 => ImageColorSpace::Gray,
        3 => ImageColorSpace::Rgb,
        4 => ImageColorSpace::Cmyk,
        _ => return None,
    };
    let alpha = components
        .iter()
        .flat_map(|c| if c.is_alpha() { Some(c) } else { None })
        .next()
        .map(|c| c.data_u8().collect::<Vec<_>>());
    let mut components_iters = image
        .components()
        .iter()
        .flat_map(|c| {
            if c.is_alpha() {
                None
            } else {
                Some(c.data_u8())
            }
        })
        .collect::<Vec<_>>();
    let mut buf = vec![];

    'outer: loop {
        for iter in &mut components_iters {
            if let Some(n) = iter.next() {
                buf.push(n);
            } else {
                break 'outer;
            }
        }
    }

    buf = if bpc == 8 {
        buf
    } else {
        scale(buf.as_slice(), bpc, cs.num_components(), width, height)?
    };

    Some(FilterResult {
        data: buf,
        image_data: Some(ImageData {
            alpha,
            color_space: cs,
            bits_per_component: bpc,
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
    let mut input = vec![0; ((width + 1) * num_components as u32 * height) as usize];
    let bit_size = BitSize::from_u8(bit_per_component)?;
    let mut writer = BitWriter::new(&mut input, bit_size)?;

    let old_max = ((1 << 8) - 1) as f32;
    let new_max = ((1 << bit_per_component) - 1) as f32;

    for bytes in data.chunks_exact(num_components as usize * width as usize) {
        for byte in bytes {
            let scaled = ((*byte as f32 / old_max) * new_max) as u16;
            writer.write(scaled)?;
        }

        writer.align();
    }

    let final_pos = writer.cur_pos();
    input.truncate(final_pos);

    Some(input)
}
