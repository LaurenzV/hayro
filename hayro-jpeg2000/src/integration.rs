//! Integration with the [image] crate

use std::{
    ffi::OsStr,
    io::{BufRead, Seek},
};

use crate::jp2::icc::{ICCColorSpace, ICCMetadata};
use crate::{ColorSpace, DecodeSettings, Image};
use ::image::error::{DecodingError, ImageFormatHint};
use ::image::{ColorType, ExtendedColorType, ImageDecoder, ImageError, ImageResult};
use image::hooks::{decoding_hook_registered, register_format_detection_hook};
use moxcms::{CmsError, ColorProfile, Layout, TransformOptions};

const CMYK_PROFILE: &[u8] = include_bytes!("../assets/CGATS001Compat-v2-micro.icc");

impl ImageDecoder for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width(), self.height())
    }

    fn color_type(&self) -> ColorType {
        let channel_count = self.color_space.num_channels();
        let has_alpha = self.has_alpha;

        match (channel_count, has_alpha) {
            (1, false) => ColorType::L8,
            (1, true) => ColorType::La8,
            (3, false) => ColorType::Rgb8,
            (3, true) => ColorType::Rgba8,
            // We convert CMYK to RGB.
            (4, false) => ColorType::Rgb8,
            (4, true) => ColorType::Rgba8,
            // We have to return something...
            _ => ColorType::Rgb8,
        }
    }

    fn original_color_type(&self) -> ExtendedColorType {
        let channel_count = self.color_space.num_channels();
        let has_alpha = self.has_alpha;
        let depth = self.original_bit_depth();
        // match logic based on color_type() above
        match (channel_count, depth, has_alpha) {
            // Grayscale
            (1, 1, false) => ExtendedColorType::L1,
            (1, 1, true) => ExtendedColorType::La1,
            (1, 2, false) => ExtendedColorType::L2,
            (1, 2, true) => ExtendedColorType::La2,
            (1, 4, false) => ExtendedColorType::L4,
            (1, 4, true) => ExtendedColorType::La4,
            (1, 8, false) => ExtendedColorType::L8,
            (1, 8, true) => ExtendedColorType::La8,
            (1, 16, false) => ExtendedColorType::L8,
            (1, 16, true) => ExtendedColorType::La8,
            // RGB
            (3, 1, false) => ExtendedColorType::Rgb1,
            (3, 1, true) => ExtendedColorType::Rgba1,
            (3, 2, false) => ExtendedColorType::Rgb2,
            (3, 2, true) => ExtendedColorType::Rgba2,
            (3, 4, false) => ExtendedColorType::Rgb4,
            (3, 4, true) => ExtendedColorType::Rgba4,
            (3, 8, false) => ExtendedColorType::Rgb8,
            (3, 8, true) => ExtendedColorType::Rgba8,
            (3, 16, false) => ExtendedColorType::Rgb8,
            (3, 16, true) => ExtendedColorType::Rgba8,
            // CMYK
            (4, 8, false) => ExtendedColorType::Cmyk8,
            (4, 16, false) => ExtendedColorType::Cmyk16,
            // CMYK with alpha is not representable
            _ => ExtendedColorType::Unknown(orig_bits_per_pixel(self)),
        }
    }

    fn icc_profile(&mut self) -> ImageResult<Option<Vec<u8>>> {
        Ok(icc_profile(self))
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        convert_inner(&self, buf)
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        convert_inner(&self, buf)
    }
}

#[doc(hidden)]
/// JPEG2000 decoder compatible with `image` decoding hook APIs that pass an `impl Read + Seek`
pub struct Jp2Decoder {
    // Lots of fields from `crate::Image` are duplicated here;
    // this is necessary because `crate::Image` borrows a slice and keeping it in the same struct
    // as `input: Vec<u8>` would create a self-referential struct that Rust cannot easily express.
    //
    // This approach is modeled after the integration of early versions of zune-jpeg into image:
    // https://docs.rs/image/0.25.6/src/image/codecs/jpeg/decoder.rs.html#27-58
    //
    // Buffering the entire input in memory is not an issue for lossy formats like JPEG.
    // The compression ratio is so high that an image that expands to hundreds of MB when decoded
    // only takes up a single-digit number of MB in a compressed form.
    input: Vec<u8>,
    width: u32,
    height: u32,
    color_type: ColorType,
    orig_color_type: ExtendedColorType,
    icc_profile: Option<Vec<u8>>,
}

impl Jp2Decoder {
    /// Create a new decoder that decodes from the stream ```r```
    pub fn new<R: BufRead + Seek>(r: R) -> ImageResult<Self> {
        let mut input = Vec::new();
        let mut r = r;
        r.read_to_end(&mut input)?;
        let image = Image::new(&input, &DecodeSettings::default())?;

        Ok(Self {
            width: image.width(),
            height: image.height(),
            color_type: image.color_type(),
            orig_color_type: image.original_color_type(),
            icc_profile: icc_profile(&image),
            input,
        })
    }
}

impl ImageDecoder for Jp2Decoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn color_type(&self) -> ColorType {
        self.color_type
    }

    fn original_color_type(&self) -> ExtendedColorType {
        self.orig_color_type
    }

    fn icc_profile(&mut self) -> ImageResult<Option<Vec<u8>>> {
        Ok(self.icc_profile.clone())
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        // we can safely .unwrap() because we've already done this on decoder creation and know this works
        let decoder = Image::new(&self.input, &DecodeSettings::default()).unwrap();
        decoder.read_image(buf)
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        // we can safely .unwrap() because we've already done this on decoder creation and know this works
        let decoder = Image::new(&self.input, &DecodeSettings::default()).unwrap();
        decoder.read_image(buf)
    }
}

/// Private convenience function for `image` integration
fn orig_bits_per_pixel(img: &Image<'_>) -> u8 {
    let mut channel_count = img.color_space().num_channels();
    if img.has_alpha {
        channel_count += 1;
    }
    channel_count * img.original_bit_depth()
}

fn icc_profile(image: &Image<'_>) -> Option<Vec<u8>> {
    match output(image) {
        Output::Unchanged { profile } => profile.map(<[u8]>::to_vec),
        _ => None,
    }
}

enum Output<'a> {
    Unchanged { profile: Option<&'a [u8]> },
    Converted { from: &'a [u8] },
    Unsupported,
}

fn output<'a>(image: &'a Image<'_>) -> Output<'a> {
    match image.color_space() {
        ColorSpace::Gray | ColorSpace::RGB => Output::Unchanged { profile: None },
        ColorSpace::CMYK => Output::Converted { from: CMYK_PROFILE },
        ColorSpace::Icc { profile, .. } => match ICCMetadata::from_data(profile) {
            Some(ICCMetadata {
                color_space: ICCColorSpace::Gray | ICCColorSpace::Rgb,
            }) => Output::Unchanged {
                profile: Some(profile),
            },
            _ => Output::Converted { from: profile },
        },
        ColorSpace::Unknown { .. } => Output::Unsupported,
    }
}

fn convert_inner(image: &Image<'_>, buf: &mut [u8]) -> ImageResult<()> {
    let mut decoder_context = crate::DecoderContext::default();

    match output(image) {
        Output::Unchanged { .. } => image.decode(&mut decoder_context)?.store_u8_into(buf),
        Output::Converted { from } => {
            let decoded = image.decode(&mut decoder_context)?.data_u8();
            if to_srgb(image, from, &decoded, buf).is_err() {
                match image.color_space().num_channels() {
                    3 => buf.copy_from_slice(&decoded),
                    4 => to_srgb(image, CMYK_PROFILE, &decoded, buf).map_err(icc_err_to_image)?,
                    _ => return Err(unsupported_color_error(image.original_color_type())),
                }
            }
        }
        Output::Unsupported => return Err(unsupported_color_error(image.original_color_type())),
    }

    Ok(())
}

fn to_srgb(
    image: &Image<'_>,
    profile: &[u8],
    decoded: &[u8],
    buf: &mut [u8],
) -> Result<(), CmsError> {
    let transform = |src_layout, dest_layout, input: &[u8], output: &mut [u8]| {
        ColorProfile::new_from_slice(profile)?
            .create_transform_8bit(
                src_layout,
                &ColorProfile::new_srgb(),
                dest_layout,
                TransformOptions::default(),
            )?
            .transform(input, output)
    };

    match (image.color_space().num_channels(), image.has_alpha()) {
        (3, false) => transform(Layout::Rgb, Layout::Rgb, decoded, buf),
        (3, true) => transform(Layout::Rgba, Layout::Rgba, decoded, buf),
        // moxcms reads four-channel CMYK through the RGBA layout.
        (4, false) => transform(Layout::Rgba, Layout::Rgb, decoded, buf),
        (4, true) => {
            // moxcms doesn't support CMYK interleaved with alpha, so we
            // need to split it.
            let (cmyk, alpha): (Vec<[u8; 4]>, Vec<u8>) = decoded
                .as_chunks::<5>()
                .0
                .iter()
                .map(|s| ([s[0], s[1], s[2], s[3]], s[4]))
                .unzip();
            let mut rgb = vec![0; cmyk.len() * 3];
            transform(Layout::Rgba, Layout::Rgb, cmyk.as_flattened(), &mut rgb)?;
            for ((out, rgb), alpha) in buf
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(rgb.as_chunks::<3>().0)
                .zip(alpha)
            {
                *out = [rgb[0], rgb[1], rgb[2], alpha];
            }
            Ok(())
        }
        _ => Err(CmsError::UnsupportedChannelConfiguration),
    }
}

impl From<crate::DecodeError> for DecodingError {
    fn from(value: crate::DecodeError) -> Self {
        let format = ImageFormatHint::Name("JPEG2000".to_owned());
        Self::new(format, value)
    }
}

impl From<crate::DecodeError> for ImageError {
    fn from(value: crate::DecodeError) -> Self {
        Self::Decoding(value.into())
    }
}

fn icc_err_to_image(err: CmsError) -> ImageError {
    let format = ImageFormatHint::Name("JPEG2000".to_owned());
    ImageError::Decoding(DecodingError::new(format, err))
}

fn unsupported_color_error(color: ExtendedColorType) -> ImageError {
    ImageError::Unsupported(image::error::UnsupportedError::from_format_and_kind(
        ImageFormatHint::Name("JPEG2000".to_owned()),
        image::error::UnsupportedErrorKind::Color(color),
    ))
}

/// Registers the decoder with the `image` crate so that non-format-specific calls such as
/// `ImageReader::open("image.jp2")?.decode()?;` work with JPEG2000 files.
///
/// Returns `true` on success, or `false` if the hook for JPEG2000 is already registered.
pub fn register_decoding_hook() -> bool {
    if decoding_hook_registered(OsStr::new("jp2")) {
        return false;
    }

    for extension in ["jp2", "jpg2", "j2k", "jpf"] {
        image::hooks::register_decoding_hook(
            extension.into(),
            Box::new(|r| Ok(Box::new(Jp2Decoder::new(r)?))),
        );
        register_format_detection_hook(extension.into(), crate::JP2_MAGIC, None);
    }

    for extension in ["j2c", "jpc"] {
        image::hooks::register_decoding_hook(
            extension.into(),
            Box::new(|r| Ok(Box::new(Jp2Decoder::new(r)?))),
        );
        register_format_detection_hook(extension.into(), crate::CODESTREAM_MAGIC, None);
    }

    true
}
