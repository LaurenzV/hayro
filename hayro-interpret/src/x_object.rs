use crate::cache::Cache;
use crate::color::{ColorComponents, ColorSpace, ToRgb};
use crate::context::Context;
use crate::device::Device;
use crate::function::{Function, interpolate};
use crate::interpret::state::ActiveTransferFunction;
use crate::{BlendMode, CacheKey, ClipPath, Image, ImageDrawProps, RasterImage, StencilImage};
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
use smallvec::{SmallVec, smallvec};
use std::borrow::Cow;
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
        match dict.get::<Name<'_>>(SUBTYPE)?.deref() {
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
    pub(crate) decoded: Cow<'a, [u8]>,
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

    if !context.begin_nested_interpretation() {
        return;
    }

    let has_oc = xobject_oc(&x_object.dict, context);
    if !context.ocg_state.is_visible() {
        if has_oc {
            context.ocg_state.end_marked_content();
        }
        context.end_nested_interpretation();
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

    device.pop_clip();

    if x_object.is_transparency_group {
        device.pop_transparency_group();
    }

    context.pop_root_transform();
    context.restore_state(device);

    if has_oc {
        context.ocg_state.end_marked_content();
    }

    context.end_nested_interpretation();
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

    let image = if x_object.is_mask {
        Image::Stencil(StencilImage {
            paint: context.get_paint(false),
            image_xobject: x_object.clone(),
        })
    } else {
        Image::Raster(RasterImage(x_object.clone()))
    };

    device.draw_image(
        image,
        ImageDrawProps {
            transform,
            soft_mask: None,
            blend_mode: BlendMode::default(),
        },
    );
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
        resolve_cs: impl FnOnce(&Name<'_>) -> Option<ColorSpace>,
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

    pub(crate) fn stream(&self) -> &Stream<'a> {
        &self.stream
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

struct DecodeContext<'a> {
    decoded: FilterResult<'a>,
    width: u32,
    height: u32,
    scale_factors: (f32, f32),
    color_space: ColorSpace,
    bits_per_component: u8,
    decode_arr: SmallVec<[(f32, f32); 4]>,
}

fn decode_context<'a>(
    obj: &ImageXObject<'a>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodeContext<'a>> {
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
    let color_key_mask = obj.stream.dict().get::<SmallVec<[u16; 4]>>(MASK);
    let mut decoded_color_key_mask = None;

    let image_data = if ctx.bits_per_component == 8
        && (ctx.color_space.is_device_gray() || ctx.color_space.is_device_rgb())
        && obj.transfer_function.is_none()
        && (is_default_decode || is_inverted_default_decode)
    {
        // This is actually the most common case, where the PDF is embedded
        // in such a way where we don't need to decode. In this case,
        // we can return the raw decoded data, which will already be in
        // RGB8/gray-scale with values between 0 and 255.
        fix_image_length(
            ctx.decoded.data.to_mut(),
            ctx.width,
            &mut height,
            0,
            &ctx.color_space,
        )?;

        if let Some(color_key_mask) = &color_key_mask {
            decoded_color_key_mask = decode_color_key_mask(
                &ctx.decoded.data,
                color_key_mask,
                ctx.width,
                height,
                &ctx.color_space,
                ctx.bits_per_component,
                ctx.scale_factors,
                obj.interpolate,
            );
        }

        if is_inverted_default_decode {
            for b in ctx.decoded.data.to_mut() {
                *b = 255 - *b;
            }
        }

        if ctx.color_space.is_device_gray() {
            Some(ImageData::Luma(LumaData {
                data: core::mem::take(&mut ctx.decoded.data).into_owned(),
                width: ctx.width,
                height,
                interpolate: obj.interpolate,
                scale_factors: ctx.scale_factors,
            }))
        } else if ctx.color_space.is_device_rgb() {
            Some(ImageData::Rgb(RgbData {
                data: core::mem::take(&mut ctx.decoded.data).into_owned(),
                width: ctx.width,
                height,
                interpolate: obj.interpolate,
                scale_factors: ctx.scale_factors,
            }))
        } else {
            unreachable!()
        }
    } else {
        let mut components = get_u8_components(
            &ctx.decoded.data,
            ctx.width,
            height,
            &ctx.color_space,
            ctx.bits_per_component,
            &ctx.decode_arr,
        )?;

        fix_image_length(
            components.to_mut(),
            ctx.width,
            &mut height,
            0,
            &ctx.color_space,
        )?;

        let mut rgb_data = get_rgb_data(
            &components,
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
            color_key_mask.as_deref(),
            decoded_color_key_mask,
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
    mut decoded_data: Cow<'_, [u8]>,
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
            for b in decoded_data.to_mut() {
                *b = 255 - *b;
            }
        }

        decoded_data.into_owned()
    } else {
        let components = get_components(
            &decoded_data,
            width,
            *height,
            color_space,
            bits_per_component,
        )?;

        let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
        let mut decoded = Vec::with_capacity(components.len());

        for pixel in components.chunks(color_space.num_components() as usize) {
            for (component, (decode_min, decode_max)) in pixel.iter().zip(decode_arr) {
                let value =
                    interpolate(*component as f32, 0.0, source_max, *decode_min, *decode_max);
                let value = if invert { 1.0 - value } else { value };
                decoded.push((value * 255.0 + 0.5) as u8);
            }
        }

        decoded
    };

    fix_image_length(&mut data, width, height, 0, color_space)?;

    Some(data)
}

fn resolve_alpha(
    obj: &ImageXObject<'_>,
    decoded: &mut FilterResult<'_>,
    color_key_mask: Option<&[u16]>,
    decoded_color_key_mask: Option<LumaData>,
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
    } else if let Some(color_key_mask) = color_key_mask {
        decoded_color_key_mask.or_else(|| {
            decode_color_key_mask(
                &decoded.data,
                color_key_mask,
                width,
                *height,
                color_space,
                bits_per_component,
                scale_factors,
                obj.interpolate,
            )
        })
    } else {
        None
    };

    Some(alpha)
}

fn decode_color_key_mask(
    data: &[u8],
    color_key_mask: &[u16],
    width: u32,
    mut height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    scale_factors: (f32, f32),
    interpolate: bool,
) -> Option<LumaData> {
    let components = get_components(data, width, height, color_space, bits_per_component)?;
    let mut mask_data = Vec::with_capacity(width as usize * height as usize);

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
        interpolate,
        scale_factors,
    })
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
    color_space.convert_values(&matte, &mut matte_rgb);

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
    decoded: &[u8],
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
    cs.convert(decoded, &mut output);

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

fn get_u8_components<'a>(
    data: &'a [u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Cow<'a, [u8]>> {
    if decode_is_identity(color_space, bits_per_component, decode) {
        return Some(Cow::Borrowed(data));
    }

    let num_components = color_space.num_components() as usize;
    let capacity = width as usize * height as usize * num_components;
    let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
    let ranges = color_space.component_ranges();
    let indexed_hival = color_space.indexed_hival();

    let decode_component = |value: u32, index: usize| {
        let component_index = index % num_components;
        let (decode_min, decode_max) = *decode.get(component_index)?;
        let decoded = interpolate(value as f32, 0.0, source_max, decode_min, decode_max);

        if let Some(hival) = indexed_hival {
            Some((decoded + 0.5).clamp(0.0, hival as f32) as u8)
        } else {
            let (range_min, range_max) = *ranges.get(component_index)?;
            let normalized = if range_min == range_max {
                0.0
            } else {
                (decoded - range_min) / (range_max - range_min)
            };
            Some((normalized * 255.0 + 0.5) as u8)
        }
    };

    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(capacity);
            let mut reader = BitReader::new(data);
            let mut index = 0;

            for _ in 0..height {
                for _ in 0..width {
                    for _ in 0..num_components {
                        // See `stream_ccit_not_enough_data`, some images seemingly don't have
                        // enough data, so we just pad with zeroes in this case.
                        let value = reader.read(bits_per_component).unwrap_or(0);
                        buf.push(decode_component(value, index)?);
                        index += 1;
                    }
                }

                reader.align();
            }

            Some(Cow::Owned(buf))
        }
        8 => Some(Cow::Owned(
            data.iter()
                .enumerate()
                .map(|(index, value)| decode_component(*value as u32, index))
                .collect::<Option<Vec<_>>>()?,
        )),
        16 => Some(Cow::Owned(
            data.chunks_exact(2)
                .enumerate()
                .map(|(index, value)| {
                    decode_component(u16::from_be_bytes([value[0], value[1]]) as u32, index)
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

fn get_components(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
) -> Option<Vec<u16>> {
    let num_components = color_space.num_components() as usize;
    let capacity = width as usize * height as usize * num_components;

    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(capacity);
            let mut reader = BitReader::new(data);

            for _ in 0..height {
                for _ in 0..width {
                    for _ in 0..num_components {
                        // See `stream_ccit_not_enough_data`, some images seemingly don't have
                        // enough data, so we just pad with zeroes in this case.
                        buf.push(reader.read(bits_per_component).unwrap_or(0) as u16);
                    }
                }

                reader.align();
            }

            Some(buf)
        }
        8 => Some(data.iter().map(|value| *value as u16).collect()),
        16 => Some(
            data.chunks_exact(2)
                .map(|value| u16::from_be_bytes([value[0], value[1]]))
                .collect(),
        ),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

fn decode_is_identity(
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> bool {
    let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
    source_max == u8::MAX as f32 && decode == color_space.component_ranges().as_slice()
}
