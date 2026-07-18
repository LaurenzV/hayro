use super::mask::decode_mask;
use super::{DecodeContext, decode_context, decode_u8_samples, fix_image_length, unpack_samples};
use crate::color::{ColorComponents, ToRgb};
use crate::x_object::image::ImageXObject;
use crate::{ImageData, LumaData, RgbData};
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use smallvec::SmallVec;

pub(crate) struct DecodedImage {
    pub(crate) image: ImageData,
    pub(crate) alpha: Option<LumaData>,
}

pub(crate) fn decode_image(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedImage> {
    ImageDecoder::new(obj, target_dimension)?.decode()
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
        let alpha = self.decode_alpha(&mut image);

        Some(DecodedImage { image, alpha })
    }

    fn decode_image(&mut self) -> Option<ImageData> {
        let is_default_decode = self.ctx.decode_arr
            == self
                .ctx
                .color_space
                .default_decode_arr(self.ctx.bits_per_component as f32);
        let is_inverted_default_decode = self.ctx.decode_arr
            == self
                .ctx
                .color_space
                .inverted_default_decode_arr(self.ctx.bits_per_component as f32);

        if self.ctx.bits_per_component == 8
            && (self.ctx.color_space.is_device_gray() || self.ctx.color_space.is_device_rgb())
            && self.obj.transfer_function.is_none()
            && (is_default_decode || is_inverted_default_decode)
        {
            self.decode_native(is_inverted_default_decode)
        } else {
            self.decode_converted()
        }
    }

    fn decode_native(&mut self, invert: bool) -> Option<ImageData> {
        // TODO: Generalize this path.

        // This is actually the most common case, where the PDF is embedded
        // in such a way where we don't need to decode. In this case,
        // we can return the raw decoded data, which will already be in
        // RGB8/gray-scale with values between 0 and 255.
        fix_image_length(
            self.ctx.decoded.data.to_mut(),
            self.ctx.width,
            &mut self.ctx.height,
            0,
            self.ctx.color_space.num_components() as usize,
        )?;

        if self.color_key_mask.is_some() {
            self.decoded_color_key_mask = self.decode_color_key_mask();
        }

        if invert {
            for b in self.ctx.decoded.data.to_mut() {
                *b = 255 - *b;
            }
        }

        if self.ctx.color_space.is_device_gray() {
            Some(ImageData::Luma(LumaData {
                data: core::mem::take(&mut self.ctx.decoded.data).into_owned(),
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            }))
        } else if self.ctx.color_space.is_device_rgb() {
            Some(ImageData::Rgb(RgbData {
                data: core::mem::take(&mut self.ctx.decoded.data).into_owned(),
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            }))
        } else {
            unreachable!()
        }
    }

    fn decode_converted(&mut self) -> Option<ImageData> {
        let mut components = decode_u8_samples(
            &self.ctx.decoded.data,
            self.ctx.width,
            self.ctx.height,
            &self.ctx.color_space,
            self.ctx.bits_per_component,
            &self.ctx.decode_arr,
        )?
        .into_owned();

        fix_image_length(
            &mut components,
            self.ctx.width,
            &mut self.ctx.height,
            0,
            self.ctx.color_space.num_components() as usize,
        )?;

        let mut rgb_data = self.convert_to_rgb(components)?;

        self.apply_transfer_function(&mut rgb_data);

        Some(ImageData::Rgb(rgb_data))
    }

    fn convert_to_rgb(&self, mut decoded: Vec<u8>) -> Option<RgbData> {
        // To prevent a panic when calling the `chunks` method.
        if self.ctx.color_space.num_components() == 0 {
            return None;
        }

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

    fn decode_alpha(&mut self, image: &mut ImageData) -> Option<LumaData> {
        if let Some((alpha, matte_rgb)) = self.resolve_matte()
            && alpha.width == self.ctx.width
            && alpha.height == self.ctx.height
        {
            unpremultiply(image, &alpha.data, &matte_rgb);

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

    fn resolve_matte(&self) -> Option<(LumaData, [u8; 3])> {
        let s_mask = self.obj.stream.dict().get::<Stream<'_>>(SMASK)?;
        let matte = s_mask.dict().get::<ColorComponents>(MATTE)?;

        if matte.len() != self.ctx.color_space.num_components() as usize {
            return None;
        }

        // In theory, matte needs to be applied in the image's original color space,
        // but we always do it in RGB for now.
        let mut matte_rgb = [0_u8; 3];
        self.ctx.color_space.convert_values(&matte, &mut matte_rgb);

        let mask_obj = ImageXObject::new_mask(&s_mask, &self.obj.warning_sink, &self.obj.cache)?;
        let alpha = decode_mask(&mask_obj, self.target_dimension)?.luma;

        Some((alpha, matte_rgb))
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

            for (component, min_max) in pixel.iter().zip(color_key_mask.chunks_exact(2)) {
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

fn unpremultiply(image: &mut ImageData, alpha: &[u8], matte_rgb: &[u8]) {
    match image {
        ImageData::Rgb(rgb) => {
            for (pixel, &a) in rgb.data.chunks_exact_mut(3).zip(alpha.iter()) {
                if a == 0 {
                    continue;
                }
                let inv_alpha = 255.0 / a as f32;
                for (c, &m) in pixel.iter_mut().zip(matte_rgb.iter()) {
                    let m = m as f32;
                    *c = (m + (*c as f32 - m) * inv_alpha) as u8;
                }
            }
        }
        ImageData::Luma(luma) => {
            let m = matte_rgb[0] as f32;
            for (c, &a) in luma.data.iter_mut().zip(alpha.iter()) {
                if a == 0 {
                    continue;
                }
                let inv_alpha = 255.0 / a as f32;
                *c = (m + (*c as f32 - m) * inv_alpha) as u8;
            }
        }
    }
}
