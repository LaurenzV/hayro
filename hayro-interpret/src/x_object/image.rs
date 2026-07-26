use super::decode::{DecodedImage, DecodedMask, decode_image, decode_mask};
use super::xobject_oc;
use crate::WarningSinkFn;
use crate::cache::Cache;
use crate::color::ColorSpace;
use crate::context::Context;
use crate::device::Device;
use crate::interpret::state::ActiveTransferFunction;
use crate::{BlendMode, CacheKey, Image, ImageDrawProps, RasterImage, StencilImage};
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Name, Object, Stream};
use kurbo::Affine;

#[derive(Clone)]
pub(crate) struct ImageXObject<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) color_space: Option<ColorSpace>,
    pub(crate) cache: Cache,
    pub(crate) interpolate: bool,
    pub(crate) kind: ImageKind,
    pub(crate) stream: Stream<'a>,
    pub(crate) transfer_function: Option<ActiveTransferFunction>,
    pub(crate) warning_sink: WarningSinkFn,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Image,
    Mask,
    StencilMask,
}

impl ImageKind {
    fn is_mask(self) -> bool {
        self != Self::Image
    }
}

impl<'a> ImageXObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name<'_>) -> Option<ColorSpace>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        Self::new_inner(
            stream,
            resolve_cs,
            warning_sink,
            cache,
            ImageKind::Image,
            transfer_function,
        )
    }

    pub(crate) fn new_mask(
        stream: &Stream<'a>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
    ) -> Option<Self> {
        Self::new_inner(stream, |_| None, warning_sink, cache, ImageKind::Mask, None)
    }

    fn new_inner(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name<'_>) -> Option<ColorSpace>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
        mut kind: ImageKind,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        let dict = stream.dict();

        let is_stencil_mask = dict
            .get::<bool>(IM)
            .or_else(|| dict.get::<bool>(IMAGE_MASK))
            .unwrap_or(false);

        if is_stencil_mask {
            kind = ImageKind::StencilMask;
        }

        let image_cs = if kind.is_mask() {
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
                // Apparently, some PDF producers also do this for normal images,
                // though the PDF spec forbids it. See https://github.com/LaurenzV/hayro/pull/1311.
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
            kind,
        })
    }

    pub(crate) fn draw<'b>(&self, context: &mut Context<'b>, device: &mut impl Device<'b>) {
        if !context.ocg_state.is_visible() {
            return;
        }

        let has_oc = xobject_oc(self.stream.dict(), context);
        if !context.ocg_state.is_visible() {
            if has_oc {
                context.ocg_state.end_marked_content();
            }
            return;
        }

        let width = self.width as f64;
        let height = self.height as f64;

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

        let has_alpha = self.has_mask();

        let mut soft_mask = std::mem::take(&mut context.get_mut().graphics_state.soft_mask);
        let blend_mode = std::mem::take(&mut context.get_mut().graphics_state.blend_mode);

        // If image has a soft mask, the soft mask from the graphics state
        // should be discarded.
        if has_alpha {
            soft_mask = None;
        }

        device.push_transparency_group(
            context.get().graphics_state.non_stroke_alpha,
            std::mem::take(&mut soft_mask),
            blend_mode,
        );

        let image = if self.kind.is_mask() {
            Image::Stencil(StencilImage {
                paint: context.get_paint(false),
                image_xobject: self.clone(),
            })
        } else {
            Image::Raster(RasterImage(self.clone()))
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

    pub(crate) fn decoded_mask(&self, target_dimension: Option<(u32, u32)>) -> Option<DecodedMask> {
        if !self.kind.is_mask() {
            return None;
        }

        decode_mask(self, target_dimension)
    }

    pub(crate) fn decoded_image(
        &self,
        target_dimension: Option<(u32, u32)>,
    ) -> Option<DecodedImage> {
        if self.kind.is_mask() {
            return None;
        }

        decode_image(self, target_dimension)
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

impl CacheKey for ImageXObject<'_> {
    fn cache_key(&self) -> u128 {
        self.stream.cache_key()
    }
}
