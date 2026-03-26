use crate::cache::Cache;
use crate::color::{ColorComponents, ColorSpace, ToRgb};
use crate::context::Context;
use crate::device::Device;
use crate::function::{Function, interpolate};
use crate::interpret::path::get_paint;
use crate::interpret::state::ActiveTransferFunction;
use crate::{BlendMode, CacheKey, ClipPath, Image, RasterImage, StencilImage};
use crate::{FillRule, InterpreterWarning, WarningSinkFn, interpret};
use crate::{ImageData, LumaData, RgbData};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::content::TypedIter;
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::stream::{FilterResult, ImageColorSpace, ImageDecodeParams};
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
    pub(crate) matrix: Affine,
    pub(crate) bbox: [f32; 4],
    is_transparency_group: bool,
    pub(crate) dict: Dict<'a>,
    resources: Dict<'a>,
}

impl<'a> FormXObject<'a> {
    pub(crate) fn new(stream: &Stream<'a>) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream.decoded().ok()?;
        let resources = dict.get::<Dict<'_>>(RESOURCES).unwrap_or_default();

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(MATRIX)
                .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        );
        let bbox = dict.get::<[f32; 4]>(BBOX)?;
        let is_transparency_group = dict.get::<Dict<'_>>(GROUP).is_some();

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

    let has_oc = xobject_oc(&x_object.dict, context);
    if !context.ocg_state.is_visible() {
        if has_oc {
            context.ocg_state.end_marked_content();
        }
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

    if has_oc {
        context.ocg_state.end_marked_content();
    }
}

pub(crate) fn draw_image_xobject<'a, 'b>(
    x_object: &ImageXObject<'b>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    if !context.ocg_state.is_visible() {
        return;
    }

    let has_oc = xobject_oc(x_object.stream.dict(), context);
    if !context.ocg_state.is_visible() {
        if has_oc {
            context.ocg_state.end_marked_content();
        }
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

    let has_alpha = x_object.has_mask();

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

    let image = if x_object.is_mask {
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

    if has_oc {
        context.ocg_state.end_marked_content();
    }
}

fn xobject_oc(dict: &Dict<'_>, context: &mut Context<'_>) -> bool {
    let Some(oc_dict) = dict.get::<Dict<'_>>(OC) else {
        return false;
    };

    if let Some(oc_ref) = dict.get_ref(OC) {
        context.ocg_state.begin_ocg(&oc_dict, oc_ref.into());
    } else {
        context.ocg_state.begin_ocmd(&oc_dict);
    }

    true
}

#[derive(Clone)]
pub(crate) struct ImageXObject<'a> {
    width: u32,
    height: u32,
    color_space: Option<ColorSpace>,
    cache: Cache,
    interpolate: bool,
    is_mask: bool,
    is_stencil_mask: bool,
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
        mut is_mask: bool,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        let dict = stream.dict();

        let is_stencil_mask = dict
            .get::<bool>(IM)
            .or_else(|| dict.get::<bool>(IMAGE_MASK))
            .unwrap_or(false);
        is_mask |= is_stencil_mask;

        let image_cs = if is_mask {
            // Masks are always single-channel.
            Some(ColorSpace::device_gray())
        } else {
            let cs_obj = dict
                .get::<Object<'_>>(CS)
                .or_else(|| dict.get::<Object<'_>>(COLORSPACE));

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
            width,
            cache: cache.clone(),
            height,
            color_space: image_cs,
            warning_sink: warning_sink.clone(),
            transfer_function,
            interpolate,
            stream: stream.clone(),
            is_mask,
            is_stencil_mask,
        })
    }

    pub(crate) fn decoded_mask(&self, target_dimension: Option<(u32, u32)>) -> Option<DecodedMask> {
        if !self.is_mask {
            return None;
        }

        decode_mask(self, target_dimension)
    }

    pub(crate) fn decoded_raster(
        &self,
        target_dimension: Option<(u32, u32)>,
    ) -> Option<DecodedRaster> {
        if self.is_mask {
            return None;
        }

        decode_raster(self, target_dimension)
    }

    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    fn has_mask(&self) -> bool {
        let dict = self.stream.dict();

        dict.contains_key(SMASK_IN_DATA) || dict.contains_key(SMASK) || dict.contains_key(MASK)
    }
}

pub(crate) struct DecodedMask {
    pub(crate) luma: LumaData,
}

pub(crate) struct DecodedRaster {
    pub(crate) image: ImageData,
    pub(crate) alpha: Option<LumaData>,
}

struct DecodeContext {
    decoded: FilterResult,
    width: u32,
    height: u32,
    scale_factors: (f32, f32),
    color_space: ColorSpace,
    bits_per_component: u8,
    decode_arr: SmallVec<[(f32, f32); 4]>,
}

fn decode_context(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodeContext> {
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
        target_dimension,
        width: obj.width,
        height: obj.height,
    };

    let decoded = obj
        .stream
        .decoded_image(&decode_params)
        .map_err(|_| (obj.warning_sink)(InterpreterWarning::ImageDecodeFailure))
        .ok()?;

    let (mut scale_x, mut scale_y) = (1.0, 1.0);

    let (width, height) = decoded
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
                    c.and_then(|c| match c {
                        ImageColorSpace::Gray => Some(ColorSpace::device_gray()),
                        ImageColorSpace::Rgb => Some(ColorSpace::device_rgb()),
                        ImageColorSpace::Cmyk => Some(ColorSpace::device_cmyk()),
                        ImageColorSpace::Unknown(_) => None,
                    })
                })
        })
        .unwrap_or(ColorSpace::device_gray());

    let fallback_bpc = if obj.is_stencil_mask { 1 } else { 8 };

    let bits_per_component = decoded
        .image_data
        .as_ref()
        .map(|i| i.bits_per_component)
        .or(dict_bpc)
        .unwrap_or(fallback_bpc);

    let decode_arr = dict
        .get::<Array<'_>>(D)
        .or_else(|| dict.get::<Array<'_>>(DECODE))
        .map(|a| a.iter::<(f32, f32)>().collect::<SmallVec<_>>())
        .unwrap_or(color_space.default_decode_arr(bits_per_component as f32));

    Some(DecodeContext {
        decoded,
        width,
        height,
        scale_factors: (scale_x, scale_y),
        color_space,
        bits_per_component,
        decode_arr,
    })
}

fn decode_mask(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedMask> {
    let ctx = decode_context(obj, target_dimension)?;
    let mut height = ctx.height;

    let data = decode_mask_bytes(
        ctx.decoded.data,
        ctx.width,
        &mut height,
        &ctx.color_space,
        ctx.bits_per_component,
        &ctx.decode_arr,
        // Note: The semantics between "normal" soft masks (i.e. masks defined in
        // the graphics state or via `Mask`/`SMask` are inverted compared to
        // stencil masks (defined via `ImageMask`). The former match the semantics
        // of normal alpha images, where 0 stands for invisible and MAX stands for
        // fully opaque. For stencil masks, it's the other way around: 1 means the
        // paint is visible, while 0 means it's invisible.
        obj.is_stencil_mask,
    )?;

    Some(DecodedMask {
        luma: LumaData {
            data,
            width: ctx.width,
            height,
            interpolate: obj.interpolate,
            scale_factors: ctx.scale_factors,
        },
    })
}

fn decode_raster(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedRaster> {
    let mut ctx = decode_context(obj, target_dimension)?;
    let mut height = ctx.height;

    let is_default_decode = ctx.decode_arr
        == ctx
            .color_space
            .default_decode_arr(ctx.bits_per_component as f32);
    let is_inverted_default_decode = ctx.decode_arr
        == ctx
            .color_space
            .inverted_default_decode_arr(ctx.bits_per_component as f32);

    let image_data = if ctx.bits_per_component == 8
        && ctx.color_space.supports_u8()
        && obj.transfer_function.is_none()
        && (is_default_decode || is_inverted_default_decode)
    {
        // This is actually the most common case, where the PDF is embedded
        // in such a way where we don't need to decode. In this case,
        // we can prevent the round-trip from f32 back to u8 and just return
        // the raw decoded data, which will already be in
        // RGB8/gray-scale with values between 0 and 255.
        fix_image_length(
            &mut ctx.decoded.data,
            ctx.width,
            &mut height,
            0,
            &ctx.color_space,
        )?;

        if is_inverted_default_decode {
            for b in &mut ctx.decoded.data {
                *b = 255 - *b;
            }
        }

        if ctx.color_space.is_device_gray() {
            Some(ImageData::Luma(LumaData {
                data: core::mem::take(&mut ctx.decoded.data),
                width: ctx.width,
                height,
                interpolate: obj.interpolate,
                scale_factors: ctx.scale_factors,
            }))
        } else {
            let mut output_buf = vec![0; ctx.width as usize * height as usize * 3];
            ctx.color_space
                .convert_u8(&ctx.decoded.data, &mut output_buf)?;

            Some(ImageData::Rgb(RgbData {
                data: output_buf,
                width: ctx.width,
                height,
                interpolate: obj.interpolate,
                scale_factors: ctx.scale_factors,
            }))
        }
    } else {
        let components = get_components(
            &ctx.decoded.data,
            ctx.width,
            height,
            &ctx.color_space,
            ctx.bits_per_component,
        )?;

        let mut f32_data = apply_decode_array(
            &components,
            &ctx.color_space,
            ctx.bits_per_component,
            &ctx.decode_arr,
        )?;

        fix_image_length(&mut f32_data, ctx.width, &mut height, 0.0, &ctx.color_space)?;

        let mut rgb_data = get_rgb_data(
            &f32_data,
            ctx.width,
            height,
            ctx.scale_factors,
            &ctx.color_space,
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

        rgb_data.map(ImageData::Rgb)
    };

    let mut image = image_data?;

    let alpha = if let Some((alpha, matte_rgb)) =
        resolve_matte(obj, &ctx.color_space, target_dimension)
        && alpha.width == ctx.width
        && alpha.height == height
    {
        unpremultiply(&mut image, &alpha.data, &matte_rgb);

        Some(alpha)
    } else {
        // Use flatten here, so in case the alpha channel is invalid we can still
        // return the main image (see PDFJS-19611).
        resolve_alpha(
            obj,
            &mut ctx.decoded,
            Some(&image),
            &ctx.color_space,
            ctx.bits_per_component,
            ctx.width,
            &mut height,
            ctx.scale_factors,
            target_dimension,
        )
        .flatten()
    };

    Some(DecodedRaster { image, alpha })
}

fn decode_mask_bytes(
    mut decoded_data: Vec<u8>,
    width: u32,
    height: &mut u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode_arr: &[(f32, f32)],
    invert: bool,
) -> Option<Vec<u8>> {
    let default_decode = color_space.default_decode_arr(bits_per_component as f32);
    let inverted_default = color_space.inverted_default_decode_arr(bits_per_component as f32);
    let fast_path = bits_per_component == 8
        && (decode_arr == default_decode.as_slice() || decode_arr == inverted_default.as_slice());

    let mut data = if fast_path {
        let should_invert = invert ^ (decode_arr == inverted_default.as_slice());
        if should_invert {
            for b in &mut decoded_data {
                *b = 255 - *b;
            }
        }

        decoded_data
    } else {
        let components = get_components(
            &decoded_data,
            width,
            *height,
            color_space,
            bits_per_component,
        )?;

        let f32_data =
            apply_decode_array(&components, color_space, bits_per_component, decode_arr)?;

        if invert {
            f32_data
                .iter()
                .map(|alpha| ((1.0 - *alpha) * 255.0 + 0.5) as u8)
                .collect()
        } else {
            f32_data
                .iter()
                .map(|alpha| (*alpha * 255.0 + 0.5) as u8)
                .collect()
        }
    };

    fix_image_length(&mut data, width, height, 0, color_space)?;

    Some(data)
}

fn resolve_alpha(
    obj: &ImageXObject<'_>,
    decoded: &mut FilterResult,
    image_data: Option<&ImageData>,
    color_space: &ColorSpace,
    bits_per_component: u8,
    width: u32,
    height: &mut u32,
    scale_factors: (f32, f32),
    target_dimension: Option<(u32, u32)>,
) -> Option<Option<LumaData>> {
    let dict = obj.stream.dict();

    let alpha = if let Some(1) = dict.get::<u8>(SMASK_IN_DATA) {
        let smask_data = decoded.image_data.as_mut().and_then(|i| i.alpha.take());

        if let Some(mut data) = smask_data {
            fix_image_length(&mut data, width, height, 0, &ColorSpace::device_gray())?;

            Some(LumaData {
                data,
                width,
                height: *height,
                interpolate: obj.interpolate,
                scale_factors,
            })
        } else {
            None
        }
        // Note: `SMASK` field takes precedence over `MASK`, so order matters here.
    } else if let Some(s_mask) = dict
        .get::<Stream<'_>>(SMASK)
        .or_else(|| dict.get::<Stream<'_>>(MASK))
    {
        let obj = ImageXObject::new(&s_mask, |_| None, &obj.warning_sink, &obj.cache, true, None)?;

        decode_mask(&obj, target_dimension).map(|decoded| decoded.luma)
    } else if let Some(color_key_mask) = dict.get::<SmallVec<[u16; 4]>>(MASK) {
        let mut mask_data = vec![];

        // TODO: Make this less ugly.
        let raw_data = match image_data {
            Some(ImageData::Luma(d)) => &d.data,
            _ => &decoded.data,
        };

        let components = get_components(raw_data, width, *height, color_space, bits_per_component)?;

        for pixel in components.chunks_exact(color_space.num_components() as usize) {
            let mut mask_val = 0;

            for (component, min_max) in pixel.iter().zip(color_key_mask.chunks_exact(2)) {
                if *component > min_max[1] || *component < min_max[0] {
                    mask_val = 255;
                }
            }

            mask_data.push(mask_val);
        }

        fix_image_length(&mut mask_data, width, height, 0, &ColorSpace::device_gray())?;

        Some(LumaData {
            data: mask_data,
            width,
            height: *height,
            interpolate: obj.interpolate,
            scale_factors,
        })
    } else {
        None
    };

    Some(alpha)
}

fn resolve_matte(
    obj: &ImageXObject<'_>,
    color_space: &ColorSpace,
    target_dimension: Option<(u32, u32)>,
) -> Option<(LumaData, [u8; 3])> {
    let dict = obj.stream.dict();
    let s_mask = dict.get::<Stream<'_>>(SMASK)?;
    let matte = s_mask.dict().get::<ColorComponents>(MATTE)?;

    if matte.len() != color_space.num_components() as usize {
        return None;
    }

    // In theory, matte needs to be applied in the image's original color space,
    // but we always do it in RGB for now.
    let mut matte_rgb = [0_u8; 3];
    color_space.convert_f32(&matte, &mut matte_rgb, false);

    let mask_obj = ImageXObject::new(&s_mask, |_| None, &obj.warning_sink, &obj.cache, true, None)?;
    let alpha = decode_mask(&mask_obj, target_dimension)?.luma;

    Some((alpha, matte_rgb))
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

fn apply_decode_array(
    components: &[u16],
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Vec<f32>> {
    let interpolate = |n: f32, d_min: f32, d_max: f32| {
        interpolate(
            n,
            0.0,
            2.0_f32.powi(bits_per_component as i32) - 1.0,
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
