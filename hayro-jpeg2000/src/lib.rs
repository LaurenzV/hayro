/*!
A memory-safe, pure-Rust JPEG 2000 decoder.

`hayro-jpeg2000` can decode both raw JPEG 2000 codestreams (`.j2c`) and images wrapped
inside the JP2 container format. The decoder supports the vast majority of features
defined in the JPEG2000 core coding system (ISO/IEC 15444-1) as well as some color
spaces from the extensions (ISO/IEC 15444-2). There are still some missing pieces
for some "obscure" features(like for example support for progression order
changes in tile-parts), but all features that actually commonly appear in real-life
images should be supported (if not, please open an issue!).

The decoder abstracts away most of the internal complexity of JPEG2000
and yields a simple 8-bit image with either greyscale, RGB, CMYK or an ICC-based
color space, which can then be processed further according to your needs.

# Example
```rust,no_run
use hayro_jpeg2000::{decode, DecodeSettings};

let data = std::fs::read("image.jp2").unwrap();
let bitmap = decode(&data, &DecodeSettings::default()).unwrap();

println!(
    "decoded {}x{} image in {:?} with alpha={}",
    bitmap.width,
    bitmap.height,
    bitmap.color_space,
    bitmap.has_alpha,
);
```

If you want to see a more comprehensive example, please take a look
at the example in [GitHub](https://github.com/LaurenzV/hayro/blob/main/hayro-jpeg2000/examples/png.rs),
which shows you the main steps needed to convert a JPEG2000 image into PNG for example.

# Testing
The decoder has been tested against 20.000+ images scraped from random PDFs
on the internet and also passes a large part of the OpenJPEG test suite. So you
can expect the crate to perform decently in terms of decoding correctness.

# Performance
A decent amount of effort has already been put into optimizing this crate
(both in terms of raw performance but also memory allocations). However, there
are some more important optimizations that have not been implemented yet, so
there is definitely still room for improvement (and I am planning on implementing
them eventually).

Overall, you should expect this crate to have worse performance than OpenJPEG,
but the difference gap should not be too large.

# Safety
By default, the crate has the `simd` feature enabled, which uses the
[`fearless_simd`](https://github.com/linebender/fearless_simd) crate to accelerate
important parts of the pipeline. If you want to eliminate any usage of unsafe
in this crate as well as its dependencies, you can simply disable this
feature, at the cost of worse decoding performance. Unsafe code is forbidden
via a crate-level attribute.
*/

#![forbid(unsafe_code)]
#![forbid(missing_docs)]

use crate::j2c::ComponentData;
use crate::jp2::cdef::{ChannelAssociation, ChannelType};
use crate::jp2::cmap::ComponentMappingType;
use crate::jp2::colr::EnumeratedColorspace;
use crate::jp2::icc::ICCMetadata;
use crate::jp2::{DecodedImage, ImageBoxes};

mod j2c;
mod jp2;
pub(crate) mod reader;

/// Settings to apply during decoding.
#[derive(Debug, Copy, Clone)]
pub struct DecodeSettings {
    /// Whether palette indices should be resolved.
    ///
    /// JPEG2000 images can be stored in two different ways. First, by storing
    /// RGB values (depending on the color space) for each pixel. Secondly, by
    /// only storing a single index for each channel, and then resolving the
    /// actual color using the index.
    ///
    /// If you disable this option, in case you have an image with palette
    /// indices, they will not be resolved, but instead a grayscale image
    /// will be returned, with each pixel value corresponding to the palette
    /// index of the location.
    pub resolve_palette_indices: bool,
    /// Whether strict mode should be enabled when decoding.
    ///
    /// It is recommended to leave this flag disabled, unless you have a
    /// specific reason not to.
    pub strict: bool,
}

impl Default for DecodeSettings {
    fn default() -> Self {
        Self {
            resolve_palette_indices: true,
            strict: false,
        }
    }
}

/// The color space of the image.
#[derive(Debug, Clone)]
pub enum ColorSpace {
    /// A grayscale image.
    Gray,
    /// An RGB image.
    RGB,
    /// A CMYK image.
    CMYK,
    /// An image based on an ICC profile.
    Icc {
        /// The raw data of the ICC profile.
        profile: Vec<u8>,
        /// The number of channels used by the ICC profile.
        num_channels: u8,
    },
}

impl ColorSpace {
    /// Return the number of expected channels for the color space.
    pub fn num_channels(&self) -> u8 {
        match self {
            ColorSpace::Gray => 1,
            ColorSpace::RGB => 3,
            ColorSpace::CMYK => 4,
            ColorSpace::Icc {
                num_channels: num_components,
                ..
            } => *num_components,
        }
    }
}

/// A bitmap storing the decoded result of the image.
pub struct Bitmap {
    /// The color space of the image.
    pub color_space: ColorSpace,
    /// The raw pixel data of the image. The result will always be in
    /// 8-bit (in case the original image had a different bit-depth,
    /// hayro-jpeg2000 always scales to 8-bit).
    ///
    /// The size is guaranteed to equal
    /// `width * height * (num_channels + (if has_alpha { 1 } else { 0 })`.
    /// Pixels are interleaved on a per-channel basis, the alpha channel always
    /// appearing as the last channel, if available.
    pub data: Vec<u8>,
    /// Whether the image has an alpha channel.
    pub has_alpha: bool,
    /// The width of the image.
    pub width: u32,
    /// The height of the image.
    pub height: u32,
    /// The original bit depth of the image. You usually don't need to do anything
    /// with this parameter, it just exists for informational purposes.
    pub original_bit_depth: u8,
}

/// Decode a JPEG2000 codestream (or a codestream wrapped in a JP2 file) into
/// a bitmap.
pub fn decode(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
    // JP2 signature box: 00 00 00 0C 6A 50 20 20
    const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
    const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";

    let mut decoded_image = if data.starts_with(JP2_MAGIC) {
        jp2::decode(data, settings)?
    } else if data.starts_with(CODESTREAM_MAGIC) {
        j2c::decode(data, settings)?
    } else {
        return Err("invalid JP2 file");
    };

    let width = decoded_image.decoded.width;
    let height = decoded_image.decoded.height;

    // Resolve palette indices.
    if settings.resolve_palette_indices {
        decoded_image.decoded.components =
            resolve_palette_indices(decoded_image.decoded.components, &decoded_image.boxes)
                .ok_or("failed to resolve palette indices")?;
    }

    // Check that we only have at most one alpha channel, and that the alpha
    // chanel is the last component.
    let mut has_alpha = false;

    if let Some(cdef) = &decoded_image.boxes.channel_definition {
        let last = cdef.channel_definitions.last().unwrap();
        has_alpha = last.channel_type == ChannelType::Opacity;

        // Sort by the channel association. Note that this will only work if
        // each component is referenced only once.
        let mut components = decoded_image
            .decoded
            .components
            .into_iter()
            .zip(
                cdef.channel_definitions
                    .iter()
                    .map(|c| match c._association {
                        ChannelAssociation::WholeImage => u16::MAX,
                        ChannelAssociation::Colour(c) => c,
                    }),
            )
            .collect::<Vec<_>>();
        components.sort_by(|c1, c2| c1.1.cmp(&c2.1));
        decoded_image.decoded.components = components.into_iter().map(|c| c.0).collect();
    }

    // Note that this is only valid if all images have the same bit depth.
    let bit_depth = decoded_image.decoded.components[0].bit_depth;

    let mut color_space = resolve_color_space(&mut decoded_image, bit_depth)?;

    // If we didn't resolve palette indices, we need to assume grayscale image.
    if !settings.resolve_palette_indices && decoded_image.boxes.palette.is_some() {
        has_alpha = false;
        color_space = ColorSpace::Gray;
    }

    // Validate the number of channels.
    if decoded_image.decoded.components.len()
        != (color_space.num_channels() + if has_alpha { 1 } else { 0 }) as usize
    {
        if !settings.strict
            && decoded_image.decoded.components.len() == color_space.num_channels() as usize + 1
            && !has_alpha
        {
            // See OPENJPEG test case orb-blue10-lin-j2k. Assume that we have an
            // alpha channel in this case.
            has_alpha = true;
        } else {
            return Err("image has too many channels");
        }
    }

    Ok(Bitmap {
        color_space,
        has_alpha,
        original_bit_depth: bit_depth,
        data: interleave_and_convert(decoded_image),
        width,
        height,
    })
}

fn interleave_and_convert(image: DecodedImage) -> Vec<u8> {
    let mut components = image.decoded.components;
    let num_components = components.len();

    let mut all_same_bit_depth = Some(components[0].bit_depth);

    for component in components.iter().skip(1) {
        if Some(component.bit_depth) != all_same_bit_depth {
            all_same_bit_depth = None;
        }
    }

    let max_len = components[0].container.len();

    if all_same_bit_depth == Some(8) && num_components <= 4 {
        // Fast path for the common case.
        match num_components {
            // Gray-scale.
            1 => components[0]
                .container
                .iter()
                .map(|v| v.round() as u8)
                .collect(),
            // Gray-scale with alpha.
            2 => {
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 2);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                }

                data
            }
            // RGB
            3 => {
                let c2 = components.pop().unwrap();
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];
                let c2 = &c2.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 3);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                    data.push(c2[i].round() as u8);
                }

                data
            }
            // RGBA or CMYK.
            4 => {
                let c3 = components.pop().unwrap();
                let c2 = components.pop().unwrap();
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];
                let c2 = &c2.container[..max_len];
                let c3 = &c3.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 4);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                    data.push(c2[i].round() as u8);
                    data.push(c3[i].round() as u8);
                }

                data
            }
            _ => unreachable!(),
        }
    } else {
        // Slow path that also requires us to scale to 8 bit.
        let mut buf = Vec::with_capacity(max_len * components.len());

        let mul_factor = ((1 << 8) - 1) as f32;

        for sample in 0..max_len {
            for channel in components.iter() {
                buf.push(
                    ((channel.container[sample] / ((1 << channel.bit_depth) - 1) as f32)
                        * mul_factor)
                        .round() as u8,
                )
            }
        }

        buf
    }
}

fn resolve_color_space(
    image: &mut DecodedImage,
    bit_depth: u8,
) -> Result<ColorSpace, &'static str> {
    let cs = match &image
        .boxes
        .color_specification
        .as_ref()
        .unwrap()
        .color_space
    {
        jp2::colr::ColorSpace::Enumerated(e) => {
            match e {
                EnumeratedColorspace::Cmyk => ColorSpace::CMYK,
                EnumeratedColorspace::Srgb => ColorSpace::RGB,
                EnumeratedColorspace::RommRgb => {
                    // Use an ICC profile to process the RommRGB color space.
                    ColorSpace::Icc {
                        profile: include_bytes!("../assets/ISO22028-2_ROMM-RGB.icc").to_vec(),
                        num_channels: 3,
                    }
                }
                // TODO: Actually implement this.
                EnumeratedColorspace::EsRgb => ColorSpace::RGB,
                EnumeratedColorspace::Greyscale => ColorSpace::Gray,
                EnumeratedColorspace::Sycc => {
                    sycc_to_rgb(&mut image.decoded.components, bit_depth)
                        .ok_or("failed to convert image from sycc to RGB")?;

                    ColorSpace::RGB
                }
                _ => return Err("unsupported JP2 image"),
            }
        }
        jp2::colr::ColorSpace::Icc(icc) => {
            if let Some(metadata) = ICCMetadata::from_data(icc) {
                ColorSpace::Icc {
                    profile: icc.clone(),
                    num_channels: metadata.color_space.num_components(),
                }
            } else {
                // See OPENJPEG test orb-blue10-lin-jp2.jp2. They seem to
                // assume RGB in this case (even though the image has 4
                // components with no opacity channel, they assume RGBA instead
                // of CMYK).
                ColorSpace::RGB
            }
        }
        jp2::colr::ColorSpace::Unknown => match image.decoded.components.len() {
            1 => ColorSpace::Gray,
            3 => ColorSpace::RGB,
            4 => ColorSpace::CMYK,
            _ => return Err("JP2 image has unsupported color space"),
        },
    };

    Ok(cs)
}

fn resolve_palette_indices(
    components: Vec<ComponentData>,
    boxes: &ImageBoxes,
) -> Option<Vec<ComponentData>> {
    let Some(palette) = boxes.palette.as_ref() else {
        // Nothing to resolve.
        return Some(components);
    };

    let mapping = boxes.component_mapping.as_ref().unwrap();
    let mut resolved = Vec::with_capacity(mapping.entries.len());

    for entry in &mapping.entries {
        let component_idx = entry.component_index as usize;
        let component = components.get(component_idx)?;

        match entry.mapping_type {
            ComponentMappingType::Direct => resolved.push(component.clone()),
            ComponentMappingType::Palette { column } => {
                let column_idx = column as usize;
                let column_info = palette.columns.get(column_idx)?;

                let mut mapped = Vec::with_capacity(component.container.len());

                for &sample in &component.container {
                    let index = sample.round() as i64;
                    let value = palette.map(index as usize, column_idx)?;
                    mapped.push(value as f32);
                }

                resolved.push(ComponentData {
                    container: mapped,
                    bit_depth: column_info.bit_depth,
                });
            }
        }
    }

    Some(resolved)
}

fn sycc_to_rgb(components: &mut [ComponentData], bit_depth: u8) -> Option<()> {
    let offset = (1u32 << (bit_depth as u32 - 1)) as f32;
    let max_value = ((1u32 << bit_depth as u32) - 1) as f32;

    let (head, _) = components.split_at_mut_checked(3)?;

    let [y, cb, cr] = head else {
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

    Some(())
}
