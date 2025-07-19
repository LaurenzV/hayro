use crate::ClipPath;
use crate::color::ColorSpace;
use crate::context::Context;
use crate::device::Device;
use crate::interpret::path::get_paint;
use crate::{FillRule, InterpreterWarning, WarningSinkFn, interpret};
use crate::{LumaData, RgbData};
use hayro_syntax::bit_reader::{BitReader, BitSize};
use hayro_syntax::content::TypedIter;
use hayro_syntax::function::interpolate;
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::stream::DecodeFailure;
use hayro_syntax::page::Resources;
use kurbo::{Affine, Rect, Shape};
use log::warn;
use smallvec::SmallVec;
use std::ops::Deref;

pub(crate) enum XObject<'a> {
    FormXObject(FormXObject<'a>),
    ImageXObject(ImageXObject<'a>),
}

impl<'a> XObject<'a> {
    pub(crate) fn new(stream: &Stream<'a>, warning_sink: &WarningSinkFn) -> Option<Self> {
        let dict = stream.dict();
        match dict.get::<Name>(SUBTYPE)?.deref() {
            IMAGE => Some(Self::ImageXObject(ImageXObject::new(
                stream,
                |_| None,
                warning_sink,
            )?)),
            FORM => Some(Self::FormXObject(FormXObject::new(stream)?)),
            _ => None,
        }
    }
}

pub(crate) struct FormXObject<'a> {
    pub(crate) decoded: Vec<u8>,
    matrix: Affine,
    bbox: [f32; 4],
    is_transparency_group: bool,
    resources: Dict<'a>,
}

impl<'a> FormXObject<'a> {
    fn new(stream: &Stream<'a>) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream.decoded().ok()?;
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
    let iter = TypedIter::new(x_object.decoded.as_ref());

    context.path_mut().truncate(0);
    context.save_state();
    context.pre_concat_affine(x_object.matrix);
    context.push_root_transform();

    device.set_transform(context.get().ctm);
    if x_object.is_transparency_group {
        device.push_transparency_group(
            context.get().non_stroke_alpha,
            std::mem::take(&mut context.get_mut().soft_mask),
        );
    }

    device.set_soft_mask(context.get().soft_mask.clone());

    device.push_clip_path(&ClipPath {
        path: Rect::new(
            x_object.bbox[0] as f64,
            x_object.bbox[1] as f64,
            x_object.bbox[2] as f64,
            x_object.bbox[3] as f64,
        )
        .to_path(0.1),
        fill: FillRule::NonZero,
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

    device.push_transparency_group(
        context.get().non_stroke_alpha,
        std::mem::take(&mut context.get_mut().soft_mask),
    );
    // TODO: If image had soft mask, the one from the context should be replaced by it.
    device.set_soft_mask(context.get().soft_mask.clone());

    if x_object.is_image_mask {
        if let Some(stencil) = x_object.alpha8() {
            device.draw_stencil_image(stencil, &get_paint(context, false));
        }
    } else if let Some(rgb_image) = x_object.rgb8() {
        device.draw_rgba_image(rgb_image, x_object.alpha8());
    }

    device.pop_transparency_group();

    context.restore_state();
}

pub(crate) struct ImageXObject<'a> {
    pub decoded: Vec<u8>,
    pub width: u32,
    pub height: u32,
    color_space: ColorSpace,
    interpolate: bool,
    decode: SmallVec<[(f32, f32); 4]>,
    is_image_mask: bool,
    data_smask: Option<Vec<u8>>,
    pub dict: Dict<'a>,
    warning_sink: WarningSinkFn,
    bits_per_component: u8,
}

impl<'a> ImageXObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name) -> Option<ColorSpace>,
        warning_sink: &WarningSinkFn,
    ) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream
            .decoded_image()
            .map_err(|e| match e {
                DecodeFailure::JpxImage => warning_sink(InterpreterWarning::JpxImage),
                _ => warning_sink(InterpreterWarning::ImageDecodeFailure),
            })
            .ok()?;
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
                .image_data
                .as_ref()
                .map(|i| i.bits_per_component)
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
                    decoded
                        .image_data
                        .as_ref()
                        .map(|i| i.color_space)
                        .map(|c| match c {
                            hayro_syntax::object::stream::ImageColorSpace::Gray => {
                                ColorSpace::device_gray()
                            }
                            hayro_syntax::object::stream::ImageColorSpace::Rgb => {
                                ColorSpace::device_rgb()
                            }
                            hayro_syntax::object::stream::ImageColorSpace::Cmyk => {
                                ColorSpace::device_cmyk()
                            }
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
            data_smask: decoded.image_data.and_then(|i| i.alpha),
            height,
            color_space,
            warning_sink: warning_sink.clone(),
            interpolate,
            decode,
            is_image_mask: image_mask,
            dict: dict.clone(),
            bits_per_component,
        })
    }

    pub(crate) fn alpha8(&self) -> Option<LumaData> {
        let data_len = self.width as usize * self.height as usize;

        if self.is_image_mask {
            let decoded = self.decode_raw()?;

            Some(LumaData {
                data: fix_image_length(
                    decoded
                        .iter()
                        .map(|alpha| ((1.0 - *alpha) * 255.0 + 0.5) as u8)
                        .collect(),
                    data_len,
                    255,
                ),
                width: self.width,
                height: self.height,
                interpolate: self.interpolate,
            })
        } else {
            let (f32_data, width, height, interpolate) =
                if let Some(1) = self.dict.get::<u8>(SMASK_IN_DATA) {
                    if let Some(data) = self.data_smask.as_ref() {
                        (
                            decode(
                                data,
                                self.width,
                                self.height,
                                &ColorSpace::device_gray(),
                                8,
                                &[(0.0, 1.0)],
                            )?,
                            self.width,
                            self.height,
                            self.interpolate,
                        )
                    } else {
                        return None;
                    }
                } else if let Some(s_mask) = self.dict.get::<Stream>(SMASK) {
                    ImageXObject::new(&s_mask, |_| None, &self.warning_sink).and_then(|s| {
                        if let Some(decoded) = s.decode_raw() {
                            Some((decoded, s.width, s.height, s.interpolate))
                        } else {
                            None
                        }
                    })?
                } else if let Some(mask) = self.dict.get::<Stream>(MASK) {
                    if let Some(obj) = ImageXObject::new(&mask, |_| None, &self.warning_sink) {
                        let mut mask_data = obj.decode_raw()?;
                        mask_data = mask_data.iter().map(|v| 1.0 - *v).collect();

                        (mask_data, obj.width, obj.height, obj.interpolate)
                    } else {
                        return None;
                    }
                } else {
                    return None;
                };

            let u8_data = fix_image_length(
                f32_data.iter().map(|v| (*v * 255.0 + 0.5) as u8).collect(),
                (width * height) as usize,
                255,
            );

            Some(LumaData {
                data: u8_data,
                width,
                height,
                interpolate,
            })
        }
    }

    pub(crate) fn rgb8(&self) -> Option<RgbData> {
        let data = if self.is_image_mask {
            return None;
        } else {
            let data_len = self.width as usize * self.height as usize * 3;

            let decoded = self
                .decode_raw()?
                .chunks(self.color_space.num_components() as usize)
                .flat_map(|v| {
                    let c = self.color_space.to_rgba(v, 1.0).to_rgba8();
                    [c[0], c[1], c[2]]
                })
                .collect::<Vec<_>>();

            fix_image_length(decoded, data_len, 0)
        };

        Some(RgbData {
            data,
            width: self.width,
            height: self.height,
            interpolate: self.interpolate,
        })
    }

    fn decode_raw(&self) -> Option<Vec<f32>> {
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

fn fix_image_length(mut image: Vec<u8>, length: usize, filler: u8) -> Vec<u8> {
    image.truncate(length);

    while image.len() < length {
        image.push(filler);
    }

    image
}

fn decode(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    mut bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Vec<f32>> {
    if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
        bits_per_component = ((data.len() as u64 * 8)
            / (width as u64 * height as u64 * color_space.num_components() as u64))
            as u8;
    }

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
        1..8 | 9..16 => {
            let mut buf = vec![];
            let bpc = BitSize::from_u8(bits_per_component)?;
            let mut reader = BitReader::new(data);

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
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            return None;
        }
    };

    let mut decoded_arr = vec![];

    for components in adjusted_components.chunks(color_space.num_components() as usize) {
        for (component, (d_min, d_max)) in components.iter().zip(decode) {
            decoded_arr.push(interpolate(*component as f32, *d_min, *d_max));
        }
    }

    Some(decoded_arr)
}
