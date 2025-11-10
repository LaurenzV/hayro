use crate::cache::Cache;
use crate::color::{ColorSpace, ToRgb};
use crate::context::Context;
use crate::device::Device;
use crate::function::{Function, interpolate};
use crate::interpret::path::get_paint;
use crate::interpret::state::ActiveTransferFunction;
use crate::{BlendMode, CacheKey, ClipPath, Image, RasterImage, StencilImage};
use crate::{FillRule, InterpreterWarning, WarningSinkFn, interpret};
use crate::{LumaData, RgbData};
use hayro_common::bit::BitReader;
use hayro_syntax::content::TypedIter;
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::stream::ImageDecodeParams;
use hayro_syntax::page::Resources;
use kurbo::{Affine, Rect, Shape};
use log::warn;
use smallvec::{SmallVec, smallvec};
use std::iter;
use std::ops::Deref;

pub(crate) enum XObject<'a> {
    FormXObject(FormXObject<'a>),
    ImageXObject(ImageXObject<'a>),
}

impl<'a> XObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        let dict = stream.dict();
        match dict.get::<Name>(SUBTYPE)?.deref() {
            IMAGE => Some(Self::ImageXObject(ImageXObject::new(
                stream,
                |_| None,
                warning_sink,
                cache,
                false,
                transfer_function,
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
    pub(crate) dict: Dict<'a>,
    resources: Dict<'a>,
}

impl<'a> FormXObject<'a> {
    pub(crate) fn new(stream: &Stream<'a>) -> Option<Self> {
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
            dict: dict.clone(),
            resources,
        })
    }
}

pub(crate) fn draw_xobject<'a>(
    x_object: &XObject<'a>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    match x_object {
        XObject::FormXObject(f) => draw_form_xobject(resources, f, context, device),
        XObject::ImageXObject(i) => {
            draw_image_xobject(i, context, device);
        }
    }
}

pub(crate) fn draw_form_xobject<'a, 'b>(
    resources: &Resources<'a>,
    x_object: &'b FormXObject<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    if !context.ocg_state.is_visible() {
        return;
    }

    let iter = TypedIter::new(x_object.decoded.as_ref());

    context.path_mut().truncate(0);
    context.save_state();
    context.pre_concat_affine(x_object.matrix);
    context.push_root_transform();

    if x_object.is_transparency_group {
        device.push_transparency_group(
            context.get().graphics_state.non_stroke_alpha,
            std::mem::take(&mut context.get_mut().graphics_state.soft_mask),
            std::mem::take(&mut context.get_mut().graphics_state.blend_mode),
        );

        context.get_mut().graphics_state.non_stroke_alpha = 1.0;
        context.get_mut().graphics_state.stroke_alpha = 1.0;
    }

    device.set_soft_mask(context.get().graphics_state.soft_mask.clone());
    device.set_blend_mode(context.get().graphics_state.blend_mode);

    device.push_clip_path(&ClipPath {
        path: context.get().ctm
            * Rect::new(
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
    context.restore_state(device);
}

pub(crate) fn draw_image_xobject<'a, 'b>(
    x_object: &ImageXObject<'b>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    if !context.ocg_state.is_visible() {
        return;
    }

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

    let has_alpha = x_object.has_alpha();

    let mut soft_mask = std::mem::take(&mut context.get_mut().graphics_state.soft_mask);
    let blend_mode = std::mem::take(&mut context.get_mut().graphics_state.blend_mode);

    // If image has smask, the soft mask from the graphics state should be discarde.
    if has_alpha {
        soft_mask = None;
    }

    device.push_transparency_group(
        context.get().graphics_state.non_stroke_alpha,
        std::mem::take(&mut soft_mask),
        blend_mode,
    );

    device.set_soft_mask(None);
    device.set_blend_mode(BlendMode::default());

    let image = if x_object.is_image_mask {
        Image::Stencil(StencilImage {
            paint: get_paint(context, false),
            image_xobject: x_object.clone(),
        })
    } else {
        Image::Raster(RasterImage(x_object.clone()))
    };

    device.draw_image(image, transform);
    device.pop_transparency_group();

    context.restore_state(device);
}

#[derive(Clone)]
pub(crate) struct ImageXObject<'a> {
    width: u32,
    height: u32,
    color_space: Option<ColorSpace>,
    cache: Cache,
    interpolate: bool,
    is_image_mask: bool,
    force_luma: bool,
    stream: Stream<'a>,
    transfer_function: Option<ActiveTransferFunction>,
    warning_sink: WarningSinkFn,
}

impl<'a> ImageXObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name) -> Option<ColorSpace>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
        force_luma: bool,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        let dict = stream.dict();

        let image_mask = dict
            .get::<bool>(IM)
            .or_else(|| dict.get::<bool>(IMAGE_MASK))
            .unwrap_or(false);
        let image_cs = if image_mask {
            Some(ColorSpace::device_gray())
        } else {
            let cs_obj = dict
                .get::<Object>(CS)
                .or_else(|| dict.get::<Object>(COLORSPACE));

            cs_obj
                .clone()
                .and_then(|c| ColorSpace::new(c, cache))
                // Inline images can also refer to color spaces by name.
                .or_else(|| {
                    cs_obj
                        .and_then(|c| c.into_name())
                        .and_then(|n| resolve_cs(&n))
                })
        };

        let interpolate = dict
            .get::<bool>(I)
            .or_else(|| dict.get::<bool>(INTERPOLATE))
            .unwrap_or(false);

        let width = dict.get::<u32>(W).or_else(|| dict.get::<u32>(WIDTH))?;
        let height = dict.get::<u32>(H).or_else(|| dict.get::<u32>(HEIGHT))?;

        if width == 0 || height == 0 {
            return None;
        }

        Some(Self {
            force_luma,
            width,
            cache: cache.clone(),
            height,
            color_space: image_cs,
            warning_sink: warning_sink.clone(),
            transfer_function,
            interpolate,
            stream: stream.clone(),
            is_image_mask: image_mask,
        })
    }

    pub(crate) fn decoded_object(&self) -> Option<DecodedImageXObject> {
        DecodedImageXObject::new(self)
    }

    fn has_alpha(&self) -> bool {
        let dict = self.stream.dict();

        self.is_image_mask
            || dict.contains_key(SMASK_IN_DATA)
            || dict.contains_key(SMASK)
            || dict.contains_key(MASK)
    }
}

pub(crate) struct DecodedImageXObject {
    pub(crate) rgb_data: Option<RgbData>,
    pub(crate) luma_data: Option<LumaData>,
}

impl DecodedImageXObject {
    fn new(obj: &ImageXObject) -> Option<Self> {
        let dict = obj.stream.dict();

        let dict_bpc = dict
            .get::<u8>(BPC)
            .or_else(|| dict.get::<u8>(BITS_PER_COMPONENT));

        let color_space = obj.color_space.clone();

        let is_indexed = obj.color_space.as_ref().is_some_and(|cs| cs.is_indexed());

        let decode_params = ImageDecodeParams {
            is_indexed,
            bpc: dict_bpc,
            num_components: color_space.as_ref().map(|c| c.num_components()),
        };

        let mut decoded = obj
            .stream
            .decoded_image(&decode_params)
            .map_err(|_| InterpreterWarning::ImageDecodeFailure)
            .ok()?;

        let (mut scale_x, mut scale_y) = (1.0, 1.0);

        let (width, mut height) = decoded
            .image_data
            .as_ref()
            .map(|d| {
                scale_x = obj.width as f32 / d.width as f32;
                scale_y = obj.height as f32 / d.height as f32;

                (d.width, d.height)
            })
            .unwrap_or((obj.width, obj.height));

        let color_space = color_space
            .or_else(|| {
                decoded
                    .image_data
                    .as_ref()
                    .map(|i| i.color_space)
                    .and_then(|c| {
                        c.map(|c| match c {
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
            })
            .unwrap_or(ColorSpace::device_gray());

        let mut bits_per_component = if obj.is_image_mask {
            1
        } else {
            decoded
                .image_data
                .as_ref()
                .map(|i| i.bits_per_component)
                .or(dict_bpc)
                .unwrap_or(8)
        };

        if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
            bits_per_component = ((decoded.data.len() as u64 * 8)
                / (width as u64 * height as u64 * color_space.num_components() as u64))
                as u8;
        }

        let is_luma = obj.is_image_mask || obj.force_luma;

        let decode_arr = dict
            .get::<Array>(D)
            .or_else(|| dict.get::<Array>(DECODE))
            .map(|a| a.iter::<(f32, f32)>().collect::<SmallVec<_>>())
            .unwrap_or(color_space.default_decode_arr(bits_per_component as f32));

        let mut luma_data = None;

        let rgb_data = if is_luma {
            let components = get_components(
                &decoded.data,
                obj.width,
                obj.height,
                &color_space,
                bits_per_component,
            )?;

            let f32_data = { decode(&components, &color_space, bits_per_component, &decode_arr)? };

            let mut data = if obj.is_image_mask {
                f32_data
                    .iter()
                    .map(|alpha| ((1.0 - *alpha) * 255.0 + 0.5) as u8)
                    .collect()
            } else {
                f32_data
                    .iter()
                    .map(|alpha| (*alpha * 255.0 + 0.5) as u8)
                    .collect()
            };

            fix_image_length(&mut data, width, &mut height, 0, &color_space)?;

            luma_data = Some(LumaData {
                data,
                width,
                height,
                interpolate: obj.interpolate,
                scale_factors: (scale_x, scale_y),
            });

            return Some(Self {
                rgb_data: None,
                luma_data,
            });
        } else if bits_per_component == 8
            && color_space.supports_u8()
            && obj.transfer_function.is_none()
            && decode_arr.as_slice()
                == color_space
                    .default_decode_arr(bits_per_component as f32)
                    .as_slice()
            && !is_luma
        {
            // This is actually the most common case, where the PDF is embedded as a 8-bit RGB color
            // and no special decode array. In this case, we can prevent the round-trip from
            // f32 back to u8 and just return the raw decoded data, which will already be in
            // RGB8 with values between 0 and 255.
            fix_image_length(&mut decoded.data, width, &mut height, 0, &color_space)?;
            let mut output_buf = vec![0; width as usize * height as usize * 3];
            color_space.convert_u8(&decoded.data, &mut output_buf)?;

            Some(RgbData {
                data: output_buf,
                width,
                height,
                interpolate: obj.interpolate,
                scale_factors: (scale_x, scale_y),
            })
        } else {
            let components = get_components(
                &decoded.data,
                obj.width,
                obj.height,
                &color_space,
                bits_per_component,
            )?;

            let mut f32_data =
                { decode(&components, &color_space, bits_per_component, &decode_arr)? };

            let width = obj.width;
            let mut height = obj.height;

            fix_image_length(&mut f32_data, width, &mut height, 0.0, &color_space)?;

            let mut rgb_data = get_rgb_data(
                &f32_data,
                width,
                height,
                (scale_x, scale_y),
                &color_space,
                obj.interpolate,
            );

            if let Some(transfer_function) = &obj.transfer_function
                && let Some(rgb_data) = &mut rgb_data
            {
                let apply_single = |data: u8, function: &Function| {
                    function
                        .eval(smallvec![data as f32 / 255.0])
                        .and_then(|v| v.first().copied())
                        .map(|v| (v * 255.0 + 0.5) as u8)
                        .unwrap_or(data)
                };

                match transfer_function {
                    ActiveTransferFunction::Single(s) => {
                        for data in &mut rgb_data.data {
                            *data = apply_single(*data, s);
                        }
                    }
                    ActiveTransferFunction::Four(f) => {
                        for data in rgb_data.data.chunks_exact_mut(3) {
                            data[0] = apply_single(data[0], &f[0]);
                            data[1] = apply_single(data[1], &f[1]);
                            data[2] = apply_single(data[2], &f[2]);
                        }
                    }
                }
            }

            rgb_data
        };

        let width = obj.width;
        let mut height = obj.height;

        if !is_luma {
            let dict = obj.stream.dict();

            luma_data = if let Some(1) = dict.get::<u8>(SMASK_IN_DATA) {
                let smask_data = decoded.image_data.and_then(|i| i.alpha);

                if let Some(mut data) = smask_data {
                    fix_image_length(&mut data, width, &mut height, 0, &ColorSpace::device_gray())?;

                    Some(LumaData {
                        data,
                        width,
                        height,
                        interpolate: obj.interpolate,
                        scale_factors: (scale_x, scale_y),
                    })
                } else {
                    None
                }
            } else if let Some(s_mask) = dict.get::<Stream>(SMASK) {
                ImageXObject::new(&s_mask, |_| None, &obj.warning_sink, &obj.cache, true, None)
                    .and_then(|s| s.decoded_object().and_then(|d| d.luma_data))
            } else if let Some(mask) = dict.get::<Stream>(MASK) {
                if let Some(obj) =
                    ImageXObject::new(&mask, |_| None, &obj.warning_sink, &obj.cache, true, None)
                {
                    obj.decoded_object().and_then(|d| d.luma_data)
                } else {
                    None
                }
            } else if let Some(color_key_mask) = dict.get::<SmallVec<[u16; 4]>>(MASK) {
                let mut mask_data = vec![];

                let width = obj.width;
                let mut height = obj.height;

                let components = get_components(
                    &decoded.data,
                    obj.width,
                    obj.height,
                    &color_space,
                    bits_per_component,
                )?;

                for pixel in components.chunks_exact(color_space.num_components() as usize) {
                    let mut mask_val = 0;

                    for (component, min_max) in pixel.iter().zip(color_key_mask.chunks_exact(2)) {
                        if *component > min_max[1] || *component < min_max[0] {
                            mask_val = 255;
                        }
                    }

                    mask_data.push(mask_val);
                }

                fix_image_length(
                    &mut mask_data,
                    width,
                    &mut height,
                    0,
                    &ColorSpace::device_gray(),
                )?;

                Some(LumaData {
                    data: mask_data,
                    width,
                    height,
                    interpolate: obj.interpolate,
                    scale_factors: (scale_x, scale_y),
                })
            } else {
                None
            };
        }

        Some(Self {
            rgb_data,
            luma_data,
        })
    }
}

fn get_rgb_data(
    decoded: &[f32],
    width: u32,
    height: u32,
    scale_factors: (f32, f32),
    cs: &ColorSpace,
    interpolate: bool,
) -> Option<RgbData> {
    // To prevent a panic when calling the `chunks` method.
    if cs.num_components() == 0 {
        return None;
    }

    let mut output = vec![0; width as usize * height as usize * 3];
    cs.convert_f32(decoded, &mut output, false);

    Some(RgbData {
        data: output,
        width,
        height,
        interpolate,
        scale_factors,
    })
}

impl CacheKey for ImageXObject<'_> {
    fn cache_key(&self) -> u128 {
        self.stream.cache_key()
    }
}

#[must_use]
fn fix_image_length<T: Copy>(
    image: &mut Vec<T>,
    width: u32,
    height: &mut u32,
    filler: T,
    cs: &ColorSpace,
) -> Option<()> {
    let row_len = width as usize * cs.num_components() as usize;

    if (row_len * *height as usize) <= image.len() {
        // Too much data (or just the right amount), truncate it.
        image.truncate(row_len * *height as usize);
    } else {
        // Too little data, adapt the height and pad.
        *height = image.len().div_ceil(row_len) as u32;

        if !image.len().is_multiple_of(row_len) {
            image.extend(iter::repeat_n(filler, row_len - (image.len() % row_len)));
        }
    }

    if width == 0 || *height == 0 {
        None
    } else {
        Some(())
    }
}

fn get_components(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
) -> Option<Vec<u16>> {
    let result = match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = vec![];
            let bpc = bits_per_component;
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
            .map(|v| u16::from_be_bytes([v[0], v[1]]))
            .collect(),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            return None;
        }
    };

    Some(result)
}

fn decode(
    components: &[u16],
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

    let mut decoded_arr = vec![];

    for pixel in components.chunks(color_space.num_components() as usize) {
        for (component, (d_min, d_max)) in pixel.iter().zip(decode) {
            decoded_arr.push(interpolate(*component as f32, *d_min, *d_max));
        }
    }

    Some(decoded_arr)
}
