use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use hayro_common::bit::BitWriter;
use hayro_jpeg2000::{ColourSpecificationMethod, EnumeratedColourspace};

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

    let mut bitmap = hayro_jpeg2000::read(data).ok()?;

    let width = bitmap.metadata.width;
    let height = bitmap.metadata.height;
    let components = &mut bitmap.channels;
    let bpc = params.bpc.unwrap_or(
        components
            .iter()
            .fold(u32::MIN, |max, c| max.max(c.bit_depth as u32)) as u8,
    );
    let cs = match components.iter().filter(|c| !c.is_alpha).count() {
        1 => ImageColorSpace::Gray,
        3 => ImageColorSpace::Rgb,
        4 => ImageColorSpace::Cmyk,
        _ => return None,
    };

    let alpha = if let Some(alpha_idx) = components.iter().position(|i| i.is_alpha) {
        let el = components.remove(alpha_idx);
        scale(&el.container, bpc, 1, width, height)
    } else {
        None
    };

    let mut buf = vec![];
    let max_len = components
        .iter()
        .map(|n| n.container.len())
        .max()
        .unwrap_or(0);

    for sample in 0..max_len {
        for channel in components.iter() {
            buf.push(channel.container[sample]);
        }
    }

    if matches!(
        bitmap
            .metadata
            .colour_specification
            .as_ref()
            .map(|spec| &spec.method),
        Some(ColourSpecificationMethod::Enumerated(
            EnumeratedColourspace::Sycc
        ))
    ) {
        let bit_depth = components.first().map(|c| c.bit_depth).unwrap_or(bpc);
        sycc_to_rgb(&mut buf, bit_depth);
    }

    let buf = scale(buf.as_slice(), bpc, cs.num_components(), width, height).unwrap();

    Some(FilterResult {
        data: buf,
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
    data: &[f32],
    bit_per_component: u8,
    num_components: u8,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let mut input = vec![
        0;
        (width as u64 * num_components as u64 * bit_per_component as u64).div_ceil(8)
            as usize
            * height as usize
    ];
    let mut writer = BitWriter::new(&mut input, bit_per_component)?;
    let max = ((1 << bit_per_component) - 1) as f32;

    for bytes in data.chunks_exact(num_components as usize * width as usize) {
        for byte in bytes {
            let scaled = byte.round().min(max) as u32;
            writer.write(scaled)?;
        }

        writer.align();
    }

    let final_pos = writer.cur_pos();
    input.truncate(final_pos);

    Some(input)
}

fn sycc_to_rgb(data: &mut [f32], bit_depth: u8) {
    let offset = (1u32 << (bit_depth as u32 - 1)) as f32;
    let max_value = ((1u32 << bit_depth as u32) - 1) as f32;

    for pixel in data.chunks_exact_mut(3) {
        let y = pixel[0];
        let cb = pixel[1] - offset;
        let cr = pixel[2] - offset;

        let mut r = y + 1.402_f32 * cr;
        let mut g = y - 0.344136_f32 * cb - 0.714136_f32 * cr;
        let mut b = y + 1.772_f32 * cb;

        r = r.clamp(0.0, max_value);
        g = g.clamp(0.0, max_value);
        b = b.clamp(0.0, max_value);

        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
    }
}
