use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use hayro_common::bit::BitWriter;
use hayro_jpeg2000::bitmap::ChannelData;
use hayro_jpeg2000::{ColourSpecificationMethod, DecodeSettings, EnumeratedColourspace};

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

    let settings = DecodeSettings {
        resolve_palette_indices: false,
    };

    let mut bitmap = hayro_jpeg2000::read(data, &settings).ok()?;

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

    let bit_depth = components.first().map(|c| c.bit_depth).unwrap_or(bpc);

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
        sycc_to_rgb(components, bit_depth);
    }

    let max_len = components
        .iter()
        .map(|n| n.container.len())
        .max()
        .unwrap_or(0);

    let u8_buf: Vec<u8> = if bit_depth == 8 && matches!(components.len(), 1 | 3 | 4) {
        // Fast path for the common case.

        match components.len() {
            1 => components[0]
                .container
                .iter()
                .map(|v| v.round() as u8)
                .collect(),
            3 => {
                let b = components.pop().unwrap();
                let g = components.pop().unwrap();
                let r = components.pop().unwrap();

                let r = &r.container[..max_len];
                let g = &g.container[..max_len];
                let b = &b.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 3);

                for i in 0..max_len {
                    data.push(r[i].round() as u8);
                    data.push(g[i].round() as u8);
                    data.push(b[i].round() as u8);
                }

                data
            }
            4 => {
                let k = components.pop().unwrap();
                let y = components.pop().unwrap();
                let m = components.pop().unwrap();
                let c = components.pop().unwrap();

                let c = &c.container[..max_len];
                let m = &m.container[..max_len];
                let y = &y.container[..max_len];
                let k = &k.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 4);

                for i in 0..max_len {
                    data.push(c[i].round() as u8);
                    data.push(m[i].round() as u8);
                    data.push(y[i].round() as u8);
                    data.push(k[i].round() as u8);
                }

                data
            }
            _ => unreachable!(),
        }
    } else {
        // First interleave the channels into a contiguous buffer.
        let mut buf = vec![0.0; max_len * components.len()];
        let mut buf_iter = buf.iter_mut();

        for sample in 0..max_len {
            for channel in components.iter() {
                *buf_iter.next().unwrap() = channel.container[sample];
            }
        }

        // Scale to the bit depth
        scale(buf.as_slice(), bpc, cs.num_components(), width, height).unwrap()
    };

    Some(FilterResult {
        data: u8_buf,
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
    if bit_per_component == 8 {
        Some(data.iter().map(|v| v.round() as u8).collect())
    } else if bit_per_component == 16 {
        Some(
            data.iter()
                .flat_map(|v| (v.round() as u16).to_be_bytes())
                .collect(),
        )
    } else {
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
}

fn sycc_to_rgb(components: &mut [ChannelData], bit_depth: u8) {
    let offset = (1u32 << (bit_depth as u32 - 1)) as f32;
    let max_value = ((1u32 << bit_depth as u32) - 1) as f32;

    let [y, cb, cr] = components else {
        unreachable!();
    };

    for ((y, cb), cr) in y
        .container
        .iter_mut()
        .zip(cb.container.iter_mut())
        .zip(cr.container.iter_mut())
    {
        *cb -= offset;
        *cr -= offset;

        let r = *y + 1.402_f32 * *cr;
        let g = *y - 0.344136_f32 * *cb - 0.714136_f32 * *cr;
        let b = *y + 1.772_f32 * *cb;

        // min + max is better than clamp in terms of performance.
        *y = r.min(max_value).max(0.0);
        *cb = g.min(max_value).max(0.0);
        *cr = b.min(max_value).max(0.0);
    }
}
