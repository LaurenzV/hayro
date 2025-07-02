use crate::clip_path::ClipPath;
use crate::color::ColorSpace;
use crate::context::Context;
use crate::device::Device;
use crate::image::{RgbaImage, StencilImage};
use crate::interpret;
use crate::interpret::path::get_paint;
use hayro_syntax::bit_reader::{BitReader, BitSize};
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::document::page::Resources;
use hayro_syntax::function::interpolate;
use hayro_syntax::object::Object;
use hayro_syntax::object::array::Array;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::name::Name;
use hayro_syntax::object::stream::Stream;
use kurbo::{Affine, Rect, Shape};
use peniko::Fill;
use smallvec::SmallVec;
use std::ops::Deref;

pub enum XObject<'a> {
    FormXObject(FormXObject<'a>),
    ImageXObject(ImageXObject<'a>),
}

impl<'a> XObject<'a> {
    pub fn new(stream: &Stream<'a>) -> Option<Self> {
        let dict = stream.dict();
        match dict.get::<Name>(SUBTYPE)?.deref() {
            IMAGE => Some(Self::ImageXObject(ImageXObject::new(stream, |_| None)?)),
            FORM => Some(Self::FormXObject(FormXObject::new(stream)?)),
            _ => unimplemented!(),
        }
    }
}

pub struct FormXObject<'a> {
    pub decoded: Vec<u8>,
    matrix: Affine,
    bbox: [f32; 4],
    is_transparency_group: bool,
    resources: Dict<'a>,
}

impl<'a> FormXObject<'a> {
    fn new(stream: &Stream<'a>) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream.decoded()?;
        let resources = dict.get::<Dict>(RESOURCES).unwrap_or_default();

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(MATRIX)
                .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        );
        let bbox = dict.get::<[f32; 4]>(BBOX)?;
        let is_transparency_group = dict.get::<Dict>(GROUP).is_some();

        Some(Self {
            decoded,
            matrix,
            is_transparency_group,
            bbox,
            resources,
        })
    }
}

pub(crate) fn draw_xobject<'a>(
    x_object: &XObject<'a>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device,
) {
    match x_object {
        XObject::FormXObject(f) => draw_form_xobject(resources, f, context, device),
        XObject::ImageXObject(i) => {
            draw_image_xobject(i, context, device);
        }
    }
}

pub(crate) fn draw_form_xobject<'a>(
    resources: &Resources<'a>,
    x_object: &FormXObject<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device,
) {
    let iter = TypedIter::new(UntypedIter::new(x_object.decoded.as_ref()));

    context.save_state();
    context.pre_concat_affine(x_object.matrix);
    context.push_root_transform();

    device.set_transform(context.get().ctm);

    if x_object.is_transparency_group {
        device.push_transparency_group(context.get().non_stroke_alpha);
    }

    device.push_clip_path(&ClipPath {
        path: Rect::new(
            x_object.bbox[0] as f64,
            x_object.bbox[1] as f64,
            x_object.bbox[2] as f64,
            x_object.bbox[3] as f64,
        )
        .to_path(0.1),
        fill: Fill::NonZero,
    });

    interpret(
        iter,
        &Resources::from_parent(x_object.resources.clone(), resources.clone()),
        context,
        device,
    );

    device.pop_clip_path();

    if x_object.is_transparency_group {
        device.pop_transparency_group();
    }

    context.pop_root_transform();
    context.restore_state();
}

pub(crate) fn draw_image_xobject(
    x_object: &ImageXObject<'_>,
    context: &mut Context<'_>,
    device: &mut impl Device,
) {
    let width = x_object.width as f64;
    let height = x_object.height as f64;

    let Some(data) = x_object.as_rgba8() else {
        return;
    };

    context.save_state();
    context.pre_concat_affine(Affine::new([
        1.0 / width,
        0.0,
        0.0,
        -1.0 / height,
        0.0,
        1.0,
    ]));
    let transform = context.get().ctm;
    device.set_transform(transform);
    device.push_transparency_group(context.get().non_stroke_alpha);

    if x_object.is_image_mask {
        let stencil = StencilImage {
            stencil_data: data,
            width: x_object.width,
            height: x_object.height,
            interpolate: x_object.interpolate,
        };

        device.draw_stencil_image(stencil, &get_paint(context, false));
    } else {
        let image = RgbaImage {
            image_data: data,
            width: x_object.width,
            height: x_object.height,
            interpolate: x_object.interpolate,
        };

        device.draw_rgba_image(image);
    }

    device.pop_transparency_group();

    context.restore_state();
}

pub struct ImageXObject<'a> {
    pub decoded: Vec<u8>,
    pub width: u32,
    pub height: u32,
    color_space: ColorSpace,
    interpolate: bool,
    decode: SmallVec<[(f32, f32); 4]>,
    is_image_mask: bool,
    data_smask: Option<Vec<u8>>,
    pub dict: Dict<'a>,
    bits_per_component: u8,
}

impl<'a> ImageXObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name) -> Option<ColorSpace>,
    ) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream.decoded_image()?;
        let interpolate = dict
            .get::<bool>(I)
            .or_else(|| dict.get::<bool>(INTERPOLATE))
            .unwrap_or(false);
        let image_mask = dict
            .get::<bool>(IM)
            .or_else(|| dict.get::<bool>(IMAGE_MASK))
            .unwrap_or(false);
        let bits_per_component = if image_mask {
            1
        } else {
            decoded
                .bits_per_component
                .or_else(|| dict.get::<u8>(BPC))
                .or_else(|| dict.get::<u8>(BITS_PER_COMPONENT))
                .unwrap_or(8)
        };
        let color_space = if image_mask {
            ColorSpace::device_gray()
        } else {
            let cs_obj = dict
                .get::<Object>(CS)
                .or_else(|| dict.get::<Object>(COLORSPACE));

            cs_obj
                .clone()
                .and_then(|c| ColorSpace::new(c))
                // Inline images can also refer to color spaces by name.
                .or_else(|| {
                    cs_obj
                        .and_then(|c| c.into_name())
                        .and_then(|n| resolve_cs(&n))
                })
                .or_else(|| {
                    decoded.color_space.map(|c| match c {
                        hayro_syntax::filter::ImageColorSpace::Gray => ColorSpace::device_gray(),
                        hayro_syntax::filter::ImageColorSpace::Rgb => ColorSpace::device_rgb(),
                        hayro_syntax::filter::ImageColorSpace::Cmyk => ColorSpace::device_cmyk(),
                    })
                })
                .unwrap_or(ColorSpace::device_gray())
        };
        let decode = dict
            .get::<Array>(D)
            .or_else(|| dict.get::<Array>(DECODE))
            .map(|a| a.iter::<(f32, f32)>().collect::<SmallVec<_>>())
            .unwrap_or(color_space.default_decode_arr(bits_per_component as f32));
        let width = dict.get::<u32>(W).or_else(|| dict.get::<u32>(WIDTH))?;
        let height = dict.get::<u32>(H).or_else(|| dict.get::<u32>(HEIGHT))?;

        Some(Self {
            decoded: decoded.data,
            width,
            data_smask: decoded.alpha,
            height,
            color_space,
            interpolate,
            decode,
            is_image_mask: image_mask,
            dict: dict.clone(),
            bits_per_component,
        })
    }

    pub fn as_rgba8(&self) -> Option<Vec<u8>> {
        fn fix(mut image: Vec<u8>, length: usize, filler: u8) -> Vec<u8> {
            image.truncate(length);

            while image.len() < length {
                image.push(filler);
            }

            image
        }

        if self.is_image_mask {
            let decoded = self.decode_raw()?;

            Some(fix(
                decoded
                    .iter()
                    .flat_map(|alpha| {
                        let alpha = ((1.0 - *alpha) * 255.0 + 0.5) as u8;
                        [0, 0, 0, alpha]
                    })
                    .collect(),
                self.width as usize * self.height as usize * 4,
                255,
            ))
        } else {
            let s_mask = if let Some(1) = self.dict.get::<u8>(SMASK_IN_DATA) {
                if let Some(data) = self.data_smask.as_ref() {
                    decode(
                        data,
                        self.width,
                        self.height,
                        &ColorSpace::device_gray(),
                        8,
                        &[(0.0, 1.0)],
                    )
                } else {
                    None
                }
            } else if let Some(s_mask) = self.dict.get::<Stream>(SMASK) {
                ImageXObject::new(&s_mask, |_| None).and_then(|s| s.decode_raw())
            } else if let Some(mask) = self.dict.get::<Stream>(MASK) {
                if let Some(obj) = ImageXObject::new(&mask, |_| None) {
                    let mut mask_data = obj.decode_raw()?;

                    // Mask doesn't necessarily have the same dimensions.
                    if obj.width != self.width || obj.height != self.height {
                        let x_factor = obj.width as f32 / self.width as f32;
                        let y_factor = obj.height as f32 / self.height as f32;
                        let mut output =
                            Vec::with_capacity(self.width as usize * self.height as usize);
                        for y in 0..self.height {
                            let y = (y as f32 * y_factor).floor() as u32;
                            for x in 0..self.width {
                                let x = (x as f32 * x_factor).floor() as u32;
                                let index = y * obj.width + x;
                                output.push(mask_data[index as usize]);
                            }
                        }

                        mask_data = output;
                    }

                    mask_data = mask_data.iter().map(|v| 1.0 - *v).collect();

                    Some(mask_data)
                } else {
                    None
                }
            } else {
                None
            };

            let s_mask =
                s_mask.unwrap_or_else(|| vec![1.0; self.width as usize * self.height as usize]);

            let decoded = self
                .decode_raw()?
                .chunks(self.color_space.num_components() as usize)
                .zip(s_mask)
                .flat_map(|(v, alpha)| self.color_space.to_rgba(v, alpha).to_rgba8().to_u8_array())
                .collect::<Vec<_>>();

            Some(fix(
                decoded,
                self.width as usize * self.height as usize * 4,
                0,
            ))
        }
    }

    pub fn decode_raw(&self) -> Option<Vec<f32>> {
        decode(
            &self.decoded,
            self.width,
            self.height,
            &self.color_space,
            self.bits_per_component,
            &self.decode,
        )
    }
}

fn decode(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Vec<f32>> {
    let interpolate = |n: f32, d_min: f32, d_max: f32| {
        interpolate(
            n,
            0.0,
            2.0f32.powi(bits_per_component as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let adjusted_components = match bits_per_component {
        1 | 2 | 4 => {
            let mut buf = vec![];
            let bpc = BitSize::from_u8(bits_per_component)?;
            let mut reader = BitReader::new(data.as_ref());

            for _ in 0..height {
                for _ in 0..width {
                    for _ in 0..color_space.num_components() {
                        // See `stream_ccit_not_enough_data`, some images seemingly don't have
                        // enough data, so we just pad with zeroes in this case.
                        let next = reader.read(bpc).unwrap_or(0) as u16;

                        buf.push(next);
                    }
                }

                reader.align();
            }

            buf
        }
        8 => data.iter().map(|v| *v as u16).collect(),
        16 => data
            .chunks(2)
            .map(|v| (u16::from_be_bytes([v[0], v[1]])))
            .collect(),
        _ => unimplemented!(),
    };

    let mut decoded_arr = vec![];

    for components in adjusted_components.chunks(color_space.num_components() as usize) {
        for (component, (d_min, d_max)) in components.iter().zip(decode) {
            decoded_arr.push(interpolate(*component as f32, *d_min, *d_max));
        }
    }

    Some(decoded_arr)
}
