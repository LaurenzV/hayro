// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fine rasterization runs the commands in each wide tile to determine the final RGBA value
//! of each pixel and pack it into the pixmap.

mod shader;

use crate::coarse::{Cmd, WideTile};
use crate::encode::EncodedPaint;
use crate::fine::shader::ShaderFiller;
use crate::mask::Mask;
use crate::paint::Paint;
use crate::tile::Tile;
use core::fmt::Debug;
use core::iter;
use kurbo::{Point, Vec2};
use peniko::{BlendMode, Compose, Mix};

pub(crate) const COLOR_COMPONENTS: usize = 4;
pub(crate) const TILE_HEIGHT_COMPONENTS: usize = Tile::HEIGHT as usize * COLOR_COMPONENTS;
#[doc(hidden)]
pub const SCRATCH_BUF_SIZE: usize =
    WideTile::WIDTH as usize * Tile::HEIGHT as usize * COLOR_COMPONENTS;

pub type ScratchBuf<F> = [F; SCRATCH_BUF_SIZE];

#[derive(Debug)]
#[doc(hidden)]
/// This is an internal struct, do not access directly.
pub struct Fine {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) wide_coords: (u16, u16),
    pub(crate) blend_buf: Vec<ScratchBuf<f32>>,
    pub(crate) color_buf: ScratchBuf<f32>,
}

impl Fine {
    /// Create a new fine rasterizer.
    pub fn new(width: u16, height: u16) -> Self {
        let blend_buf = [0.0; SCRATCH_BUF_SIZE];
        let color_buf = [0.0; SCRATCH_BUF_SIZE];

        Self {
            width,
            height,
            wide_coords: (0, 0),
            blend_buf: vec![blend_buf],
            color_buf,
        }
    }

    /// Set the coordinates of the current wide tile that is being processed (in tile units).
    pub fn set_coords(&mut self, x: u16, y: u16) {
        self.wide_coords = (x, y);
    }

    pub fn clear(&mut self, premul_color: [f32; 4]) {
        let blend_buf = self.blend_buf.last_mut().unwrap();

        if premul_color[0] == premul_color[1]
            && premul_color[1] == premul_color[2]
            && premul_color[2] == premul_color[3]
        {
            // All components are the same, so we can use memset instead.
            blend_buf.fill(premul_color[0]);
        } else {
            for z in blend_buf.chunks_exact_mut(COLOR_COMPONENTS) {
                z.copy_from_slice(&premul_color);
            }
        }
    }

    #[doc(hidden)]
    pub fn pack(&mut self, out_buf: &mut [u8]) {
        let blend_buf = self.blend_buf.last_mut().unwrap();

        pack(
            out_buf,
            blend_buf,
            self.width.into(),
            self.height.into(),
            self.wide_coords.0.into(),
            self.wide_coords.1.into(),
        );
    }

    pub(crate) fn run_cmd(&mut self, cmd: &Cmd, alphas: &[u8], paints: &[EncodedPaint]) {
        match cmd {
            Cmd::Fill(f) => {
                self.fill(
                    usize::from(f.x),
                    usize::from(f.width),
                    &f.paint,
                    f.blend_mode
                        .unwrap_or(BlendMode::new(Mix::Normal, Compose::SrcOver)),
                    paints,
                );
            }
            Cmd::AlphaFill(s) => match (s.alpha_idx, s.mask.as_ref()) {
                (Some(alpha_idx), mask) => {
                    let a_slice = &alphas[alpha_idx..];

                    if let Some(mask) = mask {
                        let start_x = (self.wide_coords.0 * WideTile::WIDTH) + s.x;
                        let start_y = self.wide_coords.1 * Tile::HEIGHT;

                        let mut mask_iter = mask_fn(mask, start_x, start_y);

                        self.strip(
                            usize::from(s.x),
                            usize::from(s.width),
                            a_slice.chunks_exact(4).map(move |e| {
                                let m = mask_iter.next().unwrap();

                                [
                                    ((e[0] as u16 * m[0] as u16) / 255) as u8,
                                    ((e[1] as u16 * m[1] as u16) / 255) as u8,
                                    ((e[2] as u16 * m[2] as u16) / 255) as u8,
                                    ((e[3] as u16 * m[3] as u16) / 255) as u8,
                                ]
                            }),
                            &s.paint,
                            s.blend_mode
                                .unwrap_or(BlendMode::new(Mix::Normal, Compose::SrcOver)),
                            paints,
                        );
                    } else {
                        self.strip(
                            usize::from(s.x),
                            usize::from(s.width),
                            a_slice.chunks_exact(4).map(|e| [e[0], e[1], e[2], e[3]]),
                            &s.paint,
                            s.blend_mode
                                .unwrap_or(BlendMode::new(Mix::Normal, Compose::SrcOver)),
                            paints,
                        );
                    }
                }
                (None, Some(mask)) => {
                    let start_x = (self.wide_coords.0 * WideTile::WIDTH) + s.x;
                    let start_y = self.wide_coords.1 * Tile::HEIGHT;

                    let mask_iter = mask_fn(mask, start_x, start_y);

                    self.strip(
                        usize::from(s.x),
                        usize::from(s.width),
                        mask_iter,
                        &s.paint,
                        s.blend_mode
                            .unwrap_or(BlendMode::new(Mix::Normal, Compose::SrcOver)),
                        paints,
                    );
                }
                (None, None) => unreachable!(),
            },
            Cmd::PushBuf => {
                self.blend_buf.push([0.0; SCRATCH_BUF_SIZE]);
            }
            Cmd::PopBuf => {
                self.blend_buf.pop();
            }
            Cmd::ClipFill(cf) => {
                self.clip_fill(cf.x as usize, cf.width as usize);
            }
            Cmd::ClipStrip(cs) => {
                let aslice = &alphas[cs.alpha_idx..];
                self.clip_strip(cs.x as usize, cs.width as usize, aslice);
            }
            Cmd::Blend(cb) => {
                self.apply_blend(*cb);
            }
            Cmd::Opacity(o) => {
                if *o != 1.0 {
                    self.blend_buf
                        .last_mut()
                        .unwrap()
                        .chunks_exact_mut(TILE_HEIGHT_COMPONENTS)
                        .for_each(|s| {
                            for c in s {
                                *c *= *o;
                            }
                        });
                }
            }
            Cmd::Mask(m) => {
                let start_x = self.wide_coords.0 * WideTile::WIDTH;
                let start_y = self.wide_coords.1 * Tile::HEIGHT;

                for (x, col) in self
                    .blend_buf
                    .last_mut()
                    .unwrap()
                    .chunks_exact_mut(TILE_HEIGHT_COMPONENTS)
                    .enumerate()
                {
                    for (y, pix) in col.chunks_exact_mut(COLOR_COMPONENTS).enumerate() {
                        let x = start_x + x as u16;
                        let y = start_y + y as u16;

                        if x < m.width() && y < m.height() {
                            let val = m.sample(x, y) as f32 / 255.0;

                            for comp in pix.iter_mut() {
                                *comp *= val;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Fill at a given x and with a width using the given paint.
    pub fn fill(
        &mut self,
        x: usize,
        width: usize,
        fill: &Paint,
        blend_mode: BlendMode,
        encoded_paints: &[EncodedPaint],
    ) {
        let blend_buf = &mut self.blend_buf.last_mut().unwrap()[x * TILE_HEIGHT_COMPONENTS..]
            [..TILE_HEIGHT_COMPONENTS * width];
        let color_buf =
            &mut self.color_buf[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];

        let start_x = self.wide_coords.0 * WideTile::WIDTH + x as u16;
        let start_y = self.wide_coords.1 * Tile::HEIGHT;

        let default_blend = blend_mode == BlendMode::new(Mix::Normal, Compose::SrcOver);

        fn fill_complex_paint(
            color_buf: &mut [f32],
            blend_buf: &mut [f32],
            has_opacities: bool,
            blend_mode: BlendMode,
            filler: impl Painter,
        ) {
            if has_opacities {
                filler.paint(color_buf);
                fill::blend(
                    blend_buf,
                    color_buf.chunks_exact(4).map(|e| [e[0], e[1], e[2], e[3]]),
                    blend_mode,
                );
            } else {
                // Similarly to solid colors we can just override the previous values
                // if all colors in the gradient are fully opaque.
                filler.paint(blend_buf);
            }
        }

        match fill {
            Paint::Solid(color) => {
                let color = color.0;

                // If color is completely opaque we can just memcopy the colors.
                if color[3] == 1.0 && default_blend {
                    for t in blend_buf.chunks_exact_mut(COLOR_COMPONENTS) {
                        t.copy_from_slice(&color);
                    }

                    return;
                }

                fill::blend(blend_buf, iter::repeat(color), blend_mode);
            }
            Paint::Indexed(paint) => {
                let encoded_paint = &encoded_paints[paint.index()];

                match encoded_paint {
                    EncodedPaint::Image(i) => {
                        let filler = ShaderFiller::new(i, start_x, start_y);
                        fill_complex_paint(color_buf, blend_buf, true, blend_mode, filler);
                    }
                    EncodedPaint::Shading(s) => {
                        let filler = ShaderFiller::new(s, start_x, start_y);
                        fill_complex_paint(color_buf, blend_buf, true, blend_mode, filler);
                    }
                    EncodedPaint::Mask(i) => {
                        let filler = ShaderFiller::new(i, start_x, start_y);
                        filler.paint(color_buf);

                        for (dest, src) in
                            blend_buf.chunks_exact_mut(4).zip(color_buf.chunks_exact(4))
                        {
                            let src = src[3];

                            for dest in dest {
                                *dest *= src;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Strip at a given x and with a width using the given paint and alpha values.
    pub fn strip(
        &mut self,
        x: usize,
        width: usize,
        alphas: impl Iterator<Item = [u8; Tile::HEIGHT as usize]>,
        fill: &Paint,
        blend_mode: BlendMode,
        paints: &[EncodedPaint],
    ) {
        let blend_buf = &mut self.blend_buf.last_mut().unwrap()[x * TILE_HEIGHT_COMPONENTS..]
            [..TILE_HEIGHT_COMPONENTS * width];
        let color_buf =
            &mut self.color_buf[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];

        let start_x = self.wide_coords.0 * WideTile::WIDTH + x as u16;
        let start_y = self.wide_coords.1 * Tile::HEIGHT;

        match fill {
            Paint::Solid(color) => {
                strip::blend(blend_buf, iter::repeat(color.0), blend_mode, alphas);
            }
            Paint::Indexed(paint) => {
                let encoded_paint = &paints[paint.index()];

                match encoded_paint {
                    EncodedPaint::Image(i) => {
                        let filler = ShaderFiller::new(i, start_x, start_y);
                        filler.paint(color_buf);

                        strip::blend(
                            blend_buf,
                            color_buf.chunks_exact(4).map(|e| [e[0], e[1], e[2], e[3]]),
                            blend_mode,
                            alphas,
                        );
                    }
                    EncodedPaint::Shading(s) => {
                        let filler = ShaderFiller::new(s, start_x, start_y);
                        filler.paint(color_buf);

                        strip::blend(
                            blend_buf,
                            color_buf.chunks_exact(4).map(|e| [e[0], e[1], e[2], e[3]]),
                            blend_mode,
                            alphas,
                        );
                    }
                    EncodedPaint::Mask(i) => {
                        let filler = ShaderFiller::new(i, start_x, start_y);
                        filler.paint(color_buf);

                        for ((dest, src), alphas) in blend_buf
                            .chunks_exact_mut(16)
                            .zip(color_buf.chunks_exact(16))
                            .zip(alphas)
                        {
                            for ((dest, src), alpha) in dest
                                .chunks_exact_mut(4)
                                .zip(src.chunks_exact(4))
                                .zip(alphas)
                            {
                                let alpha = alpha as f32 / 255.0;
                                let src = src[3];

                                for dest in dest {
                                    *dest = *dest * src * alpha;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn apply_blend(&mut self, blend_mode: BlendMode) {
        let (source_buffer, rest) = self.blend_buf.split_last_mut().unwrap();
        let target_buffer = rest.last_mut().unwrap();

        fill::blend(
            target_buffer,
            source_buffer
                .chunks_exact(4)
                .map(|e| [e[0], e[1], e[2], e[3]]),
            blend_mode,
        );
    }

    fn clip_fill(&mut self, x: usize, width: usize) {
        let (source_buffer, rest) = self.blend_buf.split_last_mut().unwrap();
        let target_buffer = rest.last_mut().unwrap();

        let source_buffer =
            &mut source_buffer[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];
        let target_buffer =
            &mut target_buffer[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];

        fill::alpha_composite(
            target_buffer,
            source_buffer
                .chunks_exact(4)
                .map(|e| [e[0], e[1], e[2], e[3]]),
        );
    }

    fn clip_strip(&mut self, x: usize, width: usize, alphas: &[u8]) {
        let (source_buffer, rest) = self.blend_buf.split_last_mut().unwrap();
        let target_buffer = rest.last_mut().unwrap();

        let source_buffer =
            &mut source_buffer[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];
        let target_buffer =
            &mut target_buffer[x * TILE_HEIGHT_COMPONENTS..][..TILE_HEIGHT_COMPONENTS * width];

        strip::alpha_composite(
            target_buffer,
            source_buffer
                .chunks_exact(4)
                .map(|e| [e[0], e[1], e[2], e[3]]),
            alphas.chunks_exact(4).map(|e| [e[0], e[1], e[2], e[3]]),
        );
    }
}

fn mask_fn<'a>(
    mask: &'a Mask,
    mut start_x: u16,
    start_y: u16,
) -> impl Iterator<Item = [u8; 4]> + 'a {
    iter::from_fn(move || {
        let s1 = mask.sample(start_x, start_y);
        let s2 = mask.sample(start_x, start_y + 1);
        let s3 = mask.sample(start_x, start_y + 2);
        let s4 = mask.sample(start_x, start_y + 3);

        start_x += 1;

        Some([s1, s2, s3, s4])
    })
}

fn pack(
    out_buf: &mut [u8],
    scratch: &ScratchBuf<f32>,
    width: usize,
    height: usize,
    x: usize,
    y: usize,
) {
    let base_ix = (y * usize::from(Tile::HEIGHT) * width + x * usize::from(WideTile::WIDTH))
        * COLOR_COMPONENTS;

    // Make sure we don't process rows outside the range of the pixmap.
    let max_height = (height - y * usize::from(Tile::HEIGHT)).min(usize::from(Tile::HEIGHT));

    for j in 0..max_height {
        let line_ix = base_ix + j * width * COLOR_COMPONENTS;

        // Make sure we don't process columns outside the range of the pixmap.
        let max_width =
            (width - x * usize::from(WideTile::WIDTH)).min(usize::from(WideTile::WIDTH));
        let target_len = max_width * COLOR_COMPONENTS;
        // This helps the compiler to understand that any access to `dest` cannot
        // be out of bounds, and thus saves corresponding checks in the for loop.
        let dest = &mut out_buf[line_ix..][..target_len];

        for i in 0..max_width {
            let src = to_rgba8(
                &scratch[(i * usize::from(Tile::HEIGHT) + j) * COLOR_COMPONENTS..]
                    [..COLOR_COMPONENTS]
                    .try_into()
                    .unwrap(),
            );
            dest[i * COLOR_COMPONENTS..][..COLOR_COMPONENTS].copy_from_slice(&src);
        }
    }
}

pub(crate) mod fill {
    // See https://www.w3.org/TR/compositing-1/#porterduffcompositingoperators for the
    // formulas.

    use crate::fine::{COLOR_COMPONENTS, TILE_HEIGHT_COMPONENTS};
    use peniko::{BlendMode, Compose, Mix};

    pub(crate) fn blend<T: Iterator<Item = [f32; COLOR_COMPONENTS]>>(
        target: &mut [f32],
        source: T,
        blend_mode: BlendMode,
    ) {
        match (blend_mode.mix, blend_mode.compose) {
            (Mix::Normal, Compose::SrcOver) => alpha_composite(target, source),
            _ => unreachable!(),
        }
    }

    pub(crate) fn alpha_composite<T: Iterator<Item = [f32; COLOR_COMPONENTS]>>(
        target: &mut [f32],
        mut source: T,
    ) {
        for strip in target.chunks_exact_mut(TILE_HEIGHT_COMPONENTS) {
            for bg_c in strip.chunks_exact_mut(COLOR_COMPONENTS) {
                let src_c = source.next().unwrap();
                for i in 0..COLOR_COMPONENTS {
                    bg_c[i] = src_c[i] + (bg_c[i] * (1.0 - src_c[3]));
                }
            }
        }
    }
}

pub(crate) mod strip {
    use crate::fine::{COLOR_COMPONENTS, TILE_HEIGHT_COMPONENTS};
    use crate::tile::Tile;
    use peniko::{BlendMode, Compose, Mix};

    pub(crate) fn blend<
        T: Iterator<Item = [f32; COLOR_COMPONENTS]>,
        A: Iterator<Item = [u8; Tile::HEIGHT as usize]>,
    >(
        target: &mut [f32],
        source: T,
        blend_mode: BlendMode,
        alphas: A,
    ) {
        match (blend_mode.mix, blend_mode.compose) {
            (Mix::Normal, Compose::SrcOver) => alpha_composite(target, source, alphas),
            _ => unreachable!(),
        }
    }

    pub(crate) fn alpha_composite<
        T: Iterator<Item = [f32; COLOR_COMPONENTS]>,
        A: Iterator<Item = [u8; Tile::HEIGHT as usize]>,
    >(
        target: &mut [f32],
        mut source: T,
        mut alphas: A,
    ) {
        for bg_c in target.chunks_exact_mut(TILE_HEIGHT_COMPONENTS) {
            let masks = alphas.next().unwrap();

            for j in 0..usize::from(Tile::HEIGHT) {
                let src_c = source.next().unwrap();
                let mask_a = masks[j] as f32 / 255.0;
                let inv_src_a_mask_a = 1.0 - (mask_a * src_c[3]);

                for i in 0..COLOR_COMPONENTS {
                    let p1 = bg_c[j * COLOR_COMPONENTS + i] * inv_src_a_mask_a;
                    let p2 = src_c[i] * mask_a;

                    bg_c[j * COLOR_COMPONENTS + i] = p1 + p2;
                }
            }
        }
    }
}

trait Painter {
    fn paint(self, target: &mut [f32]);
}

pub(crate) fn to_rgba8(c: &[f32; 4]) -> [u8; 4] {
    let mut rgba = [0u8; 4];

    for i in 0..4 {
        rgba[i] = (c[i] * 255.0 + 0.5) as u8;
    }
    rgba
}

pub(crate) fn from_rgba8(c: &[u8; 4]) -> [f32; 4] {
    let mut rgba32 = [0f32; 4];

    for i in 0..4 {
        rgba32[i] = c[i] as f32 / 255.0;
    }

    rgba32
}

/// Trait for sampling values from image-like structures
pub trait Sampler {
    fn interpolate(&self) -> bool;
    fn sample_impl(&self, pos: Point) -> [f32; 4];
    fn sample(&self, pos: Point) -> [f32; 4]
    where
        Self: Sized,
    {
        if self.interpolate() {
            sample_interpolated(self, pos)
        } else {
            self.sample_impl(pos)
        }
    }
}

pub(crate) fn sample_interpolated(sampler: &impl Sampler, pos: Point) -> [f32; 4] {
    fn fract(val: f32) -> f32 {
        val - val.floor()
    }

    let x_fract = fract(pos.x as f32 + 0.5);
    let y_fract = fract(pos.y as f32 + 0.5);

    let mut interpolated_color = [0.0_f32; 4];

    let cx = [1.0 - x_fract, x_fract];
    let cy = [1.0 - y_fract, y_fract];

    for (x_idx, x) in [-0.5, 0.5].into_iter().enumerate() {
        for (y_idx, y) in [-0.5, 0.5].into_iter().enumerate() {
            let color_sample = sampler.sample_impl(pos + Vec2::new(x, y));
            let w = cx[x_idx] * cy[y_idx];

            for (component, component_sample) in interpolated_color.iter_mut().zip(color_sample) {
                *component += w * component_sample;
            }
        }
    }

    for i in 0..COLOR_COMPONENTS {
        let f32_val = interpolated_color[i]
            .clamp(0.0, 1.0)
            .min(interpolated_color[3]);
        interpolated_color[i] = f32_val;
    }

    interpolated_color
}
