use super::mask::decode_mask;
use super::{DecodeContext, decode_context, decode_u8_samples, fix_image_length, unpack_samples};
use crate::color::{ColorComponents, ToLuma, ToRgb};
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

        let mut components = if let Some(invert) = direct_invert {
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

            core::mem::take(&mut self.ctx.decoded.data).into_owned()
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

            components
        };

        // TODO: Apply single transfer functions directly to luma.
        if self.obj.transfer_function.is_none()
            && self.ctx.color_space.to_luma(&mut components).is_some()
        {
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
