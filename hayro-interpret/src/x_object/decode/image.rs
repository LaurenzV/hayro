use super::mask::decode_mask;
use super::{DecodeContext, decode_context, decode_u8_samples, fix_image_length, unpack_samples};
use crate::color::{CmykProfile, ColorComponents, ColorSpace, ToLuma, ToRgb};
use crate::interpret::state::ActiveTransferFunction;
use crate::x_object::image::ImageXObject;
use crate::{CmykData, ImageData, LumaData, RgbData};
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use smallvec::SmallVec;

pub(crate) struct DecodedImage {
    pub(crate) image: ImageData,
    pub(crate) alpha: Option<LumaData>,
}

pub(crate) struct DecodedCmykImage {
    pub(crate) image: CmykData,
    pub(crate) alpha: Option<LumaData>,
}

pub(crate) fn decode_image(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedImage> {
    ImageDecoder::new(obj, target_dimension)?.decode()
}

pub(crate) fn decode_cmyk_image(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
    profile: Option<&CmykProfile>,
) -> Option<DecodedCmykImage> {
    ImageDecoder::new(obj, target_dimension)?.decode_cmyk(profile)
}

struct ImageDecoder<'a, 'b> {
    obj: &'a ImageXObject<'b>,
    ctx: DecodeContext<'b>,
    target_dimension: Option<(u32, u32)>,
    color_key_mask: Option<SmallVec<[u16; 4]>>,
    decoded_color_key_mask: Option<LumaData>,
}

impl<'a, 'b> ImageDecoder<'a, 'b> {
    fn new(obj: &'a ImageXObject<'b>, target_dimension: Option<(u32, u32)>) -> Option<Self> {
        let ctx = decode_context(obj, target_dimension)?;
        let color_key_mask = obj.stream.dict().get::<SmallVec<[u16; 4]>>(MASK);

        Some(Self {
            obj,
            ctx,
            target_dimension,
            color_key_mask,
            decoded_color_key_mask: None,
        })
    }

    fn decode(mut self) -> Option<DecodedImage> {
        let mut image = self.decode_image()?;
        let color_space = self.ctx.color_space.clone();
        let alpha = self.decode_alpha(|alpha, matte| {
            // In theory, matte needs to be applied in the image's original color space,
            // but we always do it in RGB for now.
            let mut matte_rgb = [0_u8; 3];
            color_space.convert_values(matte, &mut matte_rgb);

            match &mut image {
                ImageData::Rgb(rgb) => unpremultiply(&mut rgb.data, alpha, &matte_rgb),
                ImageData::Luma(luma) => unpremultiply(&mut luma.data, alpha, &matte_rgb[..1]),
            }
        });

        Some(DecodedImage { image, alpha })
    }

    fn decode_cmyk(mut self, profile: Option<&CmykProfile>) -> Option<DecodedCmykImage> {
        let (components, color_space) = if self.obj.transfer_function.is_some() {
            // Transfer functions apply to RGB, so convert the RGB image, as is done for
            // solid colors.
            match self.decode_image()? {
                ImageData::Rgb(rgb) => (rgb.data, ColorSpace::device_rgb()),
                ImageData::Luma(luma) => (luma.data, ColorSpace::device_gray()),
            }
        } else {
            (self.decode_components()?, self.ctx.color_space.clone())
        };

        let mut data = vec![0; self.ctx.width as usize * self.ctx.height as usize * 4];
        color_space.convert_cmyk(&components, &mut data, profile)?;

        let color_space = self.ctx.color_space.clone();
        let alpha = self.decode_alpha(|alpha, matte| {
            let mut matte_cmyk = [0_u8; 4];
            color_space.convert_cmyk(&color_space.encode_values(matte), &mut matte_cmyk, profile);
            unpremultiply(&mut data, alpha, &matte_cmyk);
        });

        Some(DecodedCmykImage {
            image: CmykData {
                data,
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            },
            alpha,
        })
    }

    fn decode_image(&mut self) -> Option<ImageData> {
        let mut components = self.decode_components()?;

        if self
            .obj
            .transfer_function
            .as_ref()
            .is_none_or(|t| matches!(t, ActiveTransferFunction::Single(_)))
            && self.ctx.color_space.to_luma(&mut components).is_some()
        {
            if let Some(transfer_function) = &self.obj.transfer_function {
                transfer_function.apply_to(&mut components);
            }

            return Some(ImageData::Luma(LumaData {
                data: components,
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            }));
        }

        let mut rgb_data = self.convert_to_rgb(components)?;

        self.apply_transfer_function(&mut rgb_data);

        Some(ImageData::Rgb(rgb_data))
    }

    /// Decode the components of the image in its color space, with one byte each.
    fn decode_components(&mut self) -> Option<Vec<u8>> {
        let num_components = self.ctx.color_space.num_components() as usize;

        // To prevent a panic when calling the `chunks` method.
        if num_components == 0 {
            return None;
        }

        let component_ranges = self.ctx.color_space.component_ranges();
        let default_decode = self
            .ctx
            .color_space
            .default_decode_arr(self.ctx.bits_per_component as f32);
        let inverted_default_decode = self
            .ctx
            .color_space
            .inverted_default_decode_arr(self.ctx.bits_per_component as f32);
        let inverted_component_ranges = component_ranges
            .iter()
            .map(|(min, max)| (*max, *min))
            .collect::<SmallVec<[(f32, f32); 4]>>();
        let is_indexed = self.ctx.color_space.is_indexed();

        let direct_invert = if self.ctx.bits_per_component == 8
            && (self.ctx.decode_arr == component_ranges
                || is_indexed && self.ctx.decode_arr == default_decode)
        {
            Some(false)
        } else if self.ctx.bits_per_component == 8
            && (self.ctx.decode_arr == inverted_component_ranges
                || is_indexed && self.ctx.decode_arr == inverted_default_decode)
        {
            Some(true)
        } else {
            None
        };

        if let Some(invert) = direct_invert {
            // This is actually the most common case, where the PDF is embedded
            // in such a way where we don't need to decode. In this case,
            // we can use the raw decoded component values directly.
            fix_image_length(
                self.ctx.decoded.data.to_mut(),
                self.ctx.width,
                &mut self.ctx.height,
                0,
                num_components,
            )?;

            if self.color_key_mask.is_some() {
                self.decoded_color_key_mask = self.decode_color_key_mask();
            }

            if invert {
                for value in self.ctx.decoded.data.to_mut() {
                    *value = 255 - *value;
                }
            }

            Some(core::mem::take(&mut self.ctx.decoded.data).into_owned())
        } else {
            let mut components = decode_u8_samples(
                &self.ctx.decoded.data,
                self.ctx.width,
                self.ctx.height,
                &self.ctx.color_space,
                self.ctx.bits_per_component,
                &self.ctx.decode_arr,
            )?;

            fix_image_length(
                &mut components,
                self.ctx.width,
                &mut self.ctx.height,
                0,
                num_components,
            )?;

            Some(components)
        }
    }

    fn convert_to_rgb(&self, mut decoded: Vec<u8>) -> Option<RgbData> {
        if self
            .ctx
            .color_space
            .convert_in_place(&mut decoded)
            .is_none()
        {
            let mut output = vec![0; self.ctx.width as usize * self.ctx.height as usize * 3];
            self.ctx.color_space.convert(&decoded, &mut output)?;
            decoded = output;
        }

        Some(RgbData {
            data: decoded,
            width: self.ctx.width,
            height: self.ctx.height,
            interpolate: self.obj.interpolate,
            scale_factors: self.ctx.scale_factors,
        })
    }

    fn apply_transfer_function(&self, rgb_data: &mut RgbData) {
        if let Some(transfer_function) = &self.obj.transfer_function {
            transfer_function.apply_to(&mut rgb_data.data);
        }
    }

    /// Decode the alpha channel of the image. If the image was premultiplied with a matte
    /// color, `unpremultiply` is called with the alpha channel and the matte color, to undo
    /// it.
    fn decode_alpha(&mut self, unpremultiply: impl FnOnce(&[u8], &[f32])) -> Option<LumaData> {
        if let Some((alpha, matte)) = self.resolve_matte()
            && alpha.width == self.ctx.width
            && alpha.height == self.ctx.height
        {
            unpremultiply(&alpha.data, &matte);

            return Some(alpha);
        }

        // If the alpha channel is invalid, return no alpha so the main image can
        // still be returned (see PDFJS-19611).
        let dict = self.obj.stream.dict();

        if let Some(1) = dict.get::<u8>(SMASK_IN_DATA) {
            let mut data = self
                .ctx
                .decoded
                .image_data
                .as_mut()
                .and_then(|image| image.alpha.take())?;
            fix_image_length(&mut data, self.ctx.width, &mut self.ctx.height, 0, 1)?;

            Some(LumaData {
                data,
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            })
            // Note: `SMASK` field takes precedence over `MASK`, so order matters here.
        } else if let Some(s_mask) = dict
            .get::<Stream<'_>>(SMASK)
            .or_else(|| dict.get::<Stream<'_>>(MASK))
        {
            let obj = ImageXObject::new_mask(&s_mask, &self.obj.warning_sink, &self.obj.cache)?;

            decode_mask(&obj, self.target_dimension).map(|decoded| decoded.luma)
        } else if self.color_key_mask.is_some() {
            self.decoded_color_key_mask
                .take()
                .or_else(|| self.decode_color_key_mask())
        } else {
            None
        }
    }

    fn resolve_matte(&self) -> Option<(LumaData, ColorComponents)> {
        let s_mask = self.obj.stream.dict().get::<Stream<'_>>(SMASK)?;
        let matte = s_mask.dict().get::<ColorComponents>(MATTE)?;

        if matte.len() != self.ctx.color_space.num_components() as usize {
            return None;
        }

        let mask_obj = ImageXObject::new_mask(&s_mask, &self.obj.warning_sink, &self.obj.cache)?;
        let alpha = decode_mask(&mask_obj, self.target_dimension)?.luma;

        Some((alpha, matte))
    }

    fn decode_color_key_mask(&self) -> Option<LumaData> {
        let color_key_mask = self.color_key_mask.as_deref()?;
        let num_components = self.ctx.color_space.num_components() as usize;
        let components = unpack_samples(
            &self.ctx.decoded.data,
            self.ctx.width,
            self.ctx.height,
            num_components,
            self.ctx.bits_per_component,
        )?;
        let mut data = Vec::with_capacity(self.ctx.width as usize * self.ctx.height as usize);

        for pixel in components.chunks_exact(num_components) {
            let mut value = 0;

            for (component, min_max) in pixel.iter().zip(color_key_mask.as_chunks::<2>().0) {
                if *component > min_max[1] || *component < min_max[0] {
                    value = 255;
                }
            }

            data.push(value);
        }

        let mut height = self.ctx.height;
        fix_image_length(&mut data, self.ctx.width, &mut height, 0, 1)?;

        Some(LumaData {
            data,
            width: self.ctx.width,
            height,
            interpolate: self.obj.interpolate,
            scale_factors: self.ctx.scale_factors,
        })
    }
}

/// Undo the premultiplication of `data`, which has one channel per component of `matte`,
/// with the matte color.
fn unpremultiply(data: &mut [u8], alpha: &[u8], matte: &[u8]) {
    for (pixel, &a) in data.chunks_exact_mut(matte.len()).zip(alpha.iter()) {
        if a == 0 {
            continue;
        }
        let inv_alpha = 255.0 / a as f32;
        for (c, &m) in pixel.iter_mut().zip(matte.iter()) {
            let m = m as f32;
            *c = (m + (*c as f32 - m) * inv_alpha) as u8;
        }
    }
}
