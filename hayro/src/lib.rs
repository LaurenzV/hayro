/*!
A crate for rendering PDF files.

This crate allows you to render pages of a PDF file into bitmaps. It is supposed to be relatively
lightweight, since we do not have any dependencies on the GPU. All the rendering happens on the CPU.

The ultimate goal of this crate is to be a *feature-complete* and *performant* PDF rasterizer.
With that said, we are currently still very far away from reaching that goal: So far, no effort
has been put into performance optimizations, as we are still working on implementing missing features.
However, this crate is currently the most comprehensive and feature-complete
implementation of a PDF rasterizer in pure Rust. This claim is supported by the fact that we currently
include over 1000 PDF files in our regression test suite. The majority of those have been scraped
from the `pdf.js` and `PDFBOX` test suites and therefore represent a very large and diverse sample
of PDF files.

As mentioned, there are still some serious limitations, including lack of support for
encrypted/password-protected PDF files, blending and isolation, knockout groups as well as a range
of smaller features such as color key masking. But you should be able to render the vast majority
of PDF files without too many issues.

## Safety
This crate forbids unsafe code via a crate-level attribute.

## Examples
For usage examples, see the [example](https://github.com/LaurenzV/hayro/tree/master/hayro/examples) in
the GitHub repository.

## Cargo features
This crate has one optional feature:
- `embed-fonts`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use fearless_simd::Level;
use hayro_interpret::Device;
use hayro_interpret::FillRule;
use hayro_interpret::InterpreterCache;
use hayro_interpret::InterpreterSettings;
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::util::{RectExt, TransformExt};
use hayro_interpret::{BlendMode, Context, DrawMode, DrawProps, ImageDrawProps, SoftMask};
use hayro_interpret::{ClipPath, interpret_page};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use pic_scale::Scaler;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;

pub use hayro_interpret;
pub use hayro_interpret::hayro_syntax;
pub use vello_cpu;

use vello_cpu::color::AlphaColor;
use vello_cpu::color::Srgb;
use vello_cpu::color::palette::css::TRANSPARENT;
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::peniko::{Compose, Fill, Mix};
use vello_cpu::{Mask, Pixmap, RenderContext, peniko};

mod clip;
mod glyph;
mod image;
mod mask;
mod paint;
mod path;

pub(crate) struct GlobalState {
    level: Level,
    outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
    scaler: Scaler,
}

impl GlobalState {
    pub(crate) fn new(cache: &RenderCache<'_>) -> Self {
        Self {
            level: Level::new(),
            outline_cache: cache.outline_cache.clone(),
            scaler: Scaler::new(image::RESAMPLING_FUNCTION),
        }
    }
}

pub(crate) struct Renderer<'a> {
    global: &'a GlobalState,
    pub(crate) ctx: RenderContext,
    pub(crate) inside_pattern: bool,
    pub(crate) soft_mask_cache: FxHashMap<u128, Mask>,
    pub(crate) in_type3_glyph: bool,
}

impl<'r> Renderer<'r> {
    pub(crate) fn new(
        width: u16,
        height: u16,
        settings: vello_cpu::RenderSettings,
        global: &'r GlobalState,
    ) -> Self {
        Self {
            global,
            ctx: RenderContext::new_with(width, height, settings),
            inside_pattern: false,
            soft_mask_cache: FxHashMap::default(),
            in_type3_glyph: false,
        }
    }

    fn child(&self, width: u16, height: u16) -> Self {
        Self::new(
            width,
            height,
            derive_settings(self.ctx.render_settings()),
            self.global,
        )
    }

    fn apply_draw_props(&mut self, props: &DrawProps<'_>) {
        self.ctx.set_transform(props.transform);
        self.apply_soft_mask(props.soft_mask.as_ref());
        self.ctx
            .set_blend_mode(convert_blend_mode(props.blend_mode));
    }

    fn apply_image_props(&mut self, props: &ImageDrawProps<'_>) {
        self.ctx.set_transform(props.transform);
        self.apply_soft_mask(props.soft_mask.as_ref());
        self.ctx
            .set_blend_mode(convert_blend_mode(props.blend_mode));
    }
}

impl<'a, 'r> Device<'a> for Renderer<'r> {
    fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, props: ImageDrawProps<'a>) {
        Self::draw_pdf_image(self, image, props);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        Self::push_clip_path(self, clip_path);
    }

    fn push_clip_rect(&mut self, rect: &Rect) {
        Self::push_clip_rect(self, rect);
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    ) {
        Self::push_transparency_group(self, opacity, mask, blend_mode);
    }

    fn pop_clip(&mut self) {
        Self::pop_clip(self);
    }

    fn pop_transparency_group(&mut self) {
        Self::pop_transparency_group(self);
    }

    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        Self::draw_path(self, path, props, draw_mode);
    }

    fn draw_rect(&mut self, rect: &Rect, props: DrawProps<'a>, draw_mode: &DrawMode) {
        Self::draw_rect(self, rect, props, draw_mode);
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        glyph_transform: Affine,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        Self::draw_glyph(self, glyph, glyph_transform, props, draw_mode);
    }
}

/// A cache used by the renderer.
///
/// Ideally, such a cache should be constructed once per PDF and then reused across
/// multiple render invocations on the same document.
#[derive(Clone, Default)]
pub struct RenderCache<'a> {
    pub(crate) interpreter_cache: InterpreterCache<'a>,
    pub(crate) outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
}

impl<'a> RenderCache<'a> {
    /// Create a new render cache.
    pub fn new() -> Self {
        Self {
            interpreter_cache: InterpreterCache::new(),
            outline_cache: Rc::new(RefCell::new(FxHashMap::default())),
        }
    }
}

/// Settings to apply during rendering.
#[derive(Clone, Copy)]
pub struct RenderSettings {
    /// How much the contents should be scaled into the x direction.
    pub x_scale: f32,
    /// How much the contents should be scaled into the y direction.
    pub y_scale: f32,
    /// The width of the viewport. If this is set to `None`, the width will be chosen
    /// automatically based on the scale factor and the dimensions of the PDF.
    pub width: Option<u16>,
    /// The height of the viewport. If this is set to `None`, the height will be chosen
    /// automatically based on the scale factor and the dimensions of the PDF.
    pub height: Option<u16>,
    /// The background color. Determines the color of the base
    /// rectangle during rendering to a pixmap.
    pub bg_color: AlphaColor<Srgb>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            x_scale: 1.0,
            y_scale: 1.0,
            width: None,
            height: None,
            bg_color: TRANSPARENT,
        }
    }
}

/// Render the page like [`render`], but report whether the cooperative stop
/// (`interpreter_settings.stop`) fired.
///
/// The stop is polled throughout interpretation (per operator, glyph, and
/// image) and stream/image decoding, so a fired stop ends the work early and
/// the returned pixmap is incomplete. `Ok` is a completed render; `Err` carries
/// the [`StopReason`](enough::StopReason).
pub fn render_with_stop<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Result<Pixmap, enough::StopReason> {
    let pixmap = render(page, cache, interpreter_settings, render_settings);
    interpreter_settings.stop.check()?;
    Ok(pixmap)
}

/// Render the page with the given settings to a pixmap.
pub fn render<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Pixmap {
    let (x_scale, y_scale) = (render_settings.x_scale, render_settings.y_scale);
    let (width, height) = page.render_dimensions();
    let (scaled_width, scaled_height) = ((width * x_scale) as f64, (height * y_scale) as f64);
    let initial_transform = Affine::scale_non_uniform(x_scale as f64, y_scale as f64)
        * page.initial_transform(true).to_kurbo();

    let (pix_width, pix_height) = (
        render_settings.width.unwrap_or(scaled_width.floor() as u16),
        render_settings
            .height
            .unwrap_or(scaled_height.floor() as u16),
    );
    let mut state = Context::new(
        initial_transform,
        Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        &cache.interpreter_cache,
        page.xref(),
        interpreter_settings.clone(),
    );

    let vc_settings = vello_cpu::RenderSettings {
        level: vello_cpu::Level::new(),
        num_threads: 0,
    };

    let global = GlobalState::new(cache);
    let mut device = Renderer::new(pix_width, pix_height, vc_settings, &global);

    device.ctx.set_paint(render_settings.bg_color);
    device
        .ctx
        .fill_rect(&Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64));
    let mut clip_path = page.intersected_crop_box().to_kurbo().to_path(0.1);
    clip_path.apply_affine(initial_transform);
    device.push_clip_path(&ClipPath {
        path: clip_path,
        fill: FillRule::NonZero,
    });

    device.push_transparency_group(1.0, None, BlendMode::Normal);
    interpret_page(page, &mut state, &mut device);

    device.pop_transparency_group();

    device.pop_clip();

    let mut pixmap = Pixmap::new(pix_width, pix_height);
    let mut resources = vello_cpu::Resources::default();
    device.ctx.render(&mut pixmap, &mut resources);

    pixmap
}

// Just a convenience method for testing.
#[doc(hidden)]
pub fn render_pdf(
    pdf: &Pdf,
    scale: f32,
    settings: InterpreterSettings,
    range: Option<RangeInclusive<usize>>,
) -> Option<Vec<Pixmap>> {
    let cache = RenderCache::new();
    let rendered = pdf
        .pages()
        .iter()
        .enumerate()
        .flat_map(|(idx, page)| {
            if range.clone().is_some_and(|range| !range.contains(&idx)) {
                return None;
            }

            let pixmap = render(
                page,
                &cache,
                &settings,
                &RenderSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                    ..Default::default()
                },
            );

            Some(pixmap)
        })
        .collect();

    Some(rendered)
}

pub(crate) fn derive_settings(settings: &vello_cpu::RenderSettings) -> vello_cpu::RenderSettings {
    vello_cpu::RenderSettings {
        num_threads: 0,
        ..*settings
    }
}

pub(crate) fn x_y_advances(transform: &Affine) -> (Vec2, Vec2) {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    (
        Vec2::new(x_advance.x, x_advance.y),
        Vec2::new(y_advance.x, y_advance.y),
    )
}

fn convert_fill_rule(fill_rule: FillRule) -> Fill {
    match fill_rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    }
}

fn convert_blend_mode(blend_mode: BlendMode) -> peniko::BlendMode {
    let mix = match blend_mode {
        BlendMode::Normal => Mix::Normal,
        BlendMode::Multiply => Mix::Multiply,
        BlendMode::Screen => Mix::Screen,
        BlendMode::Overlay => Mix::Overlay,
        BlendMode::Darken => Mix::Darken,
        BlendMode::Lighten => Mix::Lighten,
        BlendMode::ColorDodge => Mix::ColorDodge,
        BlendMode::ColorBurn => Mix::ColorBurn,
        BlendMode::HardLight => Mix::HardLight,
        BlendMode::SoftLight => Mix::SoftLight,
        BlendMode::Difference => Mix::Difference,
        BlendMode::Exclusion => Mix::Exclusion,
        BlendMode::Hue => Mix::Hue,
        BlendMode::Saturation => Mix::Saturation,
        BlendMode::Color => Mix::Color,
        BlendMode::Luminosity => Mix::Luminosity,
    };

    peniko::BlendMode::new(mix, Compose::SrcOver)
}
