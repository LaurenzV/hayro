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
use hayro_jpeg2000::{Image, DecodeSettings};

let data = std::fs::read("image.jp2").unwrap();
let image = Image::new(&data, &DecodeSettings::default()).unwrap();

println!(
    "{}x{} image in {:?} with alpha={}",
    image.width(),
    image.height(),
    image.color_space(),
    image.has_alpha(),
);

let bitmap = image.decode().unwrap();
```

If you want to see a more comprehensive example, please take a look
at the example in [GitHub](https://github.com/LaurenzV/hayro/blob/main/hayro-jpeg2000/examples/png.rs),
which shows you the main steps needed to convert a JPEG2000 image into PNG for example.

# Testing
The decoder has been tested against 20.000+ images scraped from random PDFs
on the internet and also passes a large part of the `OpenJPEG` test suite. So you
can expect the crate to perform decently in terms of decoding correctness.

# Performance
A decent amount of effort has already been put into optimizing this crate
(both in terms of raw performance but also memory allocations). However, there
are some more important optimizations that have not been implemented yet, so
there is definitely still room for improvement (and I am planning on implementing
them eventually).

Overall, you should expect this crate to have worse performance than `OpenJPEG`,
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

use crate::j2c::{ComponentData, DecodedCodestream, Header};
use crate::jp2::cdef::{ChannelAssociation, ChannelType};
use crate::jp2::cmap::ComponentMappingType;
use crate::jp2::colr::{CieLab, EnumeratedColorspace};
use crate::jp2::icc::ICCMetadata;
use crate::jp2::{DecodedImage, ImageBoxes};

#[cfg(feature = "image")]
mod image;
mod j2c;
mod jp2;
pub(crate) mod reader;
mod util;

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
    /// A hint for the target resolution that the image should be decoded at.
    pub target_resolution: Option<(u32, u32)>,
}

impl Default for DecodeSettings {
    fn default() -> Self {
        Self {
            resolve_palette_indices: true,
            strict: false,
            target_resolution: None,
        }
    }
}

/// A JPEG2000 image or codestream.
pub struct Image<'a> {
    /// The codestream containing the data to decode.
    pub(crate) codestream: &'a [u8],
    /// The header of the J2C codestream.
    pub(crate) header: Header<'a>,
    /// The JP2 boxes of the image. In the case of a raw codestream, we
    /// will synthesize the necessary boxes.
    pub(crate) boxes: ImageBoxes,
    /// Settings that should be applied during decoding.
    pub(crate) settings: DecodeSettings,
    /// Whether the image has an alpha channel.
    pub(crate) has_alpha: bool,
    /// The color space of the image.
    pub(crate) color_space: ColorSpace,
}

impl<'a> Image<'a> {
    /// Try to create a new JPEG2000 image from the given data.
    pub fn new(data: &'a [u8], settings: &DecodeSettings) -> Result<Self, &'static str> {
        // JP2 signature box: 00 00 00 0C 6A 50 20 20
        const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
        // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
        const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";

        if data.starts_with(JP2_MAGIC) {
            jp2::parse(data, *settings)
        } else if data.starts_with(CODESTREAM_MAGIC) {
            j2c::parse(data, settings)
        } else {
            Err("invalid JPEG2000 file")
        }
    }

    /// Whether the image has an alpha channel.
    pub fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    /// The color space of the image.
    pub fn color_space(&self) -> &ColorSpace {
        &self.color_space
    }

    /// The width of the image.
    pub fn width(&self) -> u32 {
        self.header.size_data.image_width()
    }

    /// The height of the image.
    pub fn height(&self) -> u32 {
        self.header.size_data.image_height()
    }

    /// The original bit depth of the image. You usually don't need to do anything
    /// with this parameter, it just exists for informational purposes.
    pub fn original_bit_depth(&self) -> u8 {
        // Note that this only works if all components have the same precision.
        self.header.component_infos[0].size_info.precision
    }

    /// Decode the image.
    pub fn decode(&self) -> Result<Vec<u8>, &'static str> {
        let buffer_size = self.width() as usize
            * self.height() as usize
            * (self.color_space.num_channels() as usize + if self.has_alpha { 1 } else { 0 });
        let mut buf = vec![0; buffer_size];
        self.decode_into(&mut buf)?;

        Ok(buf)
    }

    /// Decode the image into the given buffer. The buffer must have the correct
    /// size.
    pub(crate) fn decode_into(&self, buf: &mut [u8]) -> Result<(), &'static str> {
        let settings = &self.settings;
        let mut decoded_image =
            j2c::decode(self.codestream, &self.header).map(move |data| DecodedImage {
                decoded: DecodedCodestream { components: data },
                boxes: self.boxes.clone(),
            })?;

        // Resolve palette indices.
        if settings.resolve_palette_indices {
            decoded_image.decoded.components =
                resolve_palette_indices(decoded_image.decoded.components, &decoded_image.boxes)
                    .ok_or("failed to resolve palette indices")?;
        }

        if let Some(cdef) = &decoded_image.boxes.channel_definition {
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
        convert_color_space(&mut decoded_image, bit_depth)?;

        interleave_and_convert(decoded_image, buf);

        Ok(())
    }
}

pub(crate) fn resolve_alpha_and_color_space(
    boxes: &ImageBoxes,
    header: &Header<'_>,
    settings: &DecodeSettings,
) -> Result<(ColorSpace, bool), &'static str> {
    let num_components = header.component_infos.len();

    let mut has_alpha = false;

    if let Some(cdef) = &boxes.channel_definition {
        let last = cdef.channel_definitions.last().unwrap();
        has_alpha = last.channel_type == ChannelType::Opacity;
    }

    let mut color_space = get_color_space(boxes, num_components)?;

    // If we didn't resolve palette indices, we need to assume grayscale image.
    if !settings.resolve_palette_indices && boxes.palette.is_some() {
        has_alpha = false;
        color_space = ColorSpace::Gray;
    }

    // Validate the number of channels.
    if boxes.palette.is_none()
        && header.component_infos.len()
            != (color_space.num_channels() + if has_alpha { 1 } else { 0 }) as usize
    {
        if !settings.strict
            && header.component_infos.len() == color_space.num_channels() as usize + 1
            && !has_alpha
        {
            // See OPENJPEG test case orb-blue10-lin-j2k. Assume that we have an
            // alpha channel in this case.
            has_alpha = true;
        } else {
            return Err("image has too many channels");
        }
    }

    Ok((color_space, has_alpha))
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
    /// An unknown color space.
    Unknown {
        /// The number of channels of the color space.
        num_channels: u8,
    },
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
            Self::Gray => 1,
            Self::RGB => 3,
            Self::CMYK => 4,
            Self::Unknown { num_channels } => *num_channels,
            Self::Icc {
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

fn interleave_and_convert(image: DecodedImage, buf: &mut [u8]) {
    let mut components = image.decoded.components;
    let num_components = components.len();

    let mut all_same_bit_depth = Some(components[0].bit_depth);

    for component in components.iter().skip(1) {
        if Some(component.bit_depth) != all_same_bit_depth {
            all_same_bit_depth = None;
        }
    }

    let max_len = components[0].container.len();

    let mut output_iter = buf.iter_mut();

    if all_same_bit_depth == Some(8) && num_components <= 4 {
        // Fast path for the common case.
        match num_components {
            // Gray-scale.
            1 => {
                for (output, input) in
                    output_iter.zip(components[0].container.iter().map(|v| v.round() as u8))
                {
                    *output = input;
                }
            }
            // Gray-scale with alpha.
            2 => {
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];

                for i in 0..max_len {
                    *output_iter.next().unwrap() = c0[i].round() as u8;
                    *output_iter.next().unwrap() = c1[i].round() as u8;
                }
            }
            // RGB
            3 => {
                let c2 = components.pop().unwrap();
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];
                let c2 = &c2.container[..max_len];

                for i in 0..max_len {
                    *output_iter.next().unwrap() = c0[i].round() as u8;
                    *output_iter.next().unwrap() = c1[i].round() as u8;
                    *output_iter.next().unwrap() = c2[i].round() as u8;
                }
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

                for i in 0..max_len {
                    *output_iter.next().unwrap() = c0[i].round() as u8;
                    *output_iter.next().unwrap() = c1[i].round() as u8;
                    *output_iter.next().unwrap() = c2[i].round() as u8;
                    *output_iter.next().unwrap() = c3[i].round() as u8;
                }
            }
            _ => unreachable!(),
        }
    } else {
        // Slow path that also requires us to scale to 8 bit.
        let mul_factor = ((1 << 8) - 1) as f32;

        for sample in 0..max_len {
            for channel in components.iter() {
                *output_iter.next().unwrap() = ((channel.container[sample]
                    / ((1 << channel.bit_depth) - 1) as f32)
                    * mul_factor)
                    .round() as u8;
            }
        }
    }
}

fn convert_color_space(image: &mut DecodedImage, bit_depth: u8) -> Result<(), &'static str> {
    if let jp2::colr::ColorSpace::Enumerated(e) = &image
        .boxes
        .color_specification
        .as_ref()
        .unwrap()
        .color_space
    {
        match e {
            EnumeratedColorspace::Sycc => {
                sycc_to_rgb(&mut image.decoded.components, bit_depth)
                    .ok_or("failed to convert image from sycc to RGB")?;
            }
            EnumeratedColorspace::CieLab(cielab) => {
                cielab_to_rgb(&mut image.decoded.components, bit_depth, cielab)
                    .ok_or("failed to convert image from LAB to RGB")?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn get_color_space(boxes: &ImageBoxes, num_components: usize) -> Result<ColorSpace, &'static str> {
    let cs = match &boxes
        .color_specification
        .as_ref()
        .ok_or("image doesn't have color space definition")?
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
                EnumeratedColorspace::EsRgb => ColorSpace::RGB,
                EnumeratedColorspace::Greyscale => ColorSpace::Gray,
                EnumeratedColorspace::Sycc => ColorSpace::RGB,
                EnumeratedColorspace::CieLab(_) => ColorSpace::Icc {
                    profile: include_bytes!("../assets/LAB.icc").to_vec(),
                    num_channels: 3,
                },
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
        jp2::colr::ColorSpace::Unknown => match num_components {
            1 => ColorSpace::Gray,
            3 => ColorSpace::RGB,
            4 => ColorSpace::CMYK,
            _ => ColorSpace::Unknown {
                num_channels: num_components as u8,
            },
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

fn cielab_to_rgb(components: &mut [ComponentData], bit_depth: u8, lab: &CieLab) -> Option<()> {
    let (head, _) = components.split_at_mut_checked(3)?;

    let [l, a, b] = head else {
        unreachable!();
    };

    let prec0 = l.bit_depth;
    let prec1 = a.bit_depth;
    let prec2 = b.bit_depth;

    // Table M.29bis – Default Offset Values and Encoding of Offsets for the CIELab Colourspace.
    // Signed values aren't handled.
    let rl = lab.rl.unwrap_or(100);
    let ra = lab.ra.unwrap_or(170);
    let rb = lab.ra.unwrap_or(200);
    let ol = lab.ol.unwrap_or(0);
    let oa = lab.oa.unwrap_or(1 << (bit_depth - 1));
    let ob = lab
        .ob
        .unwrap_or((1 << (bit_depth - 2)) + (1 << (bit_depth - 3)));

    // Copied from OpenJPEG.
    let min_l = -((rl * ol) as f32) / ((1 << prec0) - 1) as f32;
    let max_l = min_l + rl as f32;
    let min_a = -((ra * oa) as f32) / ((1 << prec1) - 1) as f32;
    let max_a = min_a + ra as f32;
    let min_b = -((rb * ob) as f32) / ((1 << prec2) - 1) as f32;
    let max_b = min_b + rb as f32;

    let bit_max = (1_u32 << bit_depth) - 1;

    // Note that we are not doing the actual conversion with the ICC profile yet,
    // just decoding the raw LAB values.
    // We leave applying the ICC profile to the user.
    for ((l, a), b) in l
        .container
        .iter_mut()
        .zip(a.container.iter_mut())
        .zip(b.container.iter_mut())
    {
        *l = min_l + *l * (max_l - min_l) / ((1 << prec0) - 1) as f32;
        *a = min_a + *a * (max_a - min_a) / ((1 << prec1) - 1) as f32;
        *b = min_b + *b * (max_b - min_b) / ((1 << prec2) - 1) as f32;

        // Make sure we are in the range [0.0, 2ˆbit_depth - 1].
        *l *= bit_max as f32 / 100.0;
        *a = (*a + 128.0) * bit_max as f32 / 255.0;
        *b = (*b + 128.0) * bit_max as f32 / 255.0;
    }

    Some(())
}

fn sycc_to_rgb(components: &mut [ComponentData], bit_depth: u8) -> Option<()> {
    let offset = (1_u32 << (bit_depth as u32 - 1)) as f32;
    let max_value = ((1_u32 << bit_depth as u32) - 1) as f32;

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
