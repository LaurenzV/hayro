// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub(crate) mod image;
pub(crate) mod shading;

use crate::encode::image::EncodedImage;
use crate::encode::shading::EncodedShading;
use crate::fine::Sampler;
use crate::paint::Paint;
use kurbo::{Affine, Point, Vec2};

#[derive(Debug)]
pub struct Shader<T: Sampler> {
    pub(crate) transform: Affine,
    pub(crate) x_advance: Vec2,
    pub(crate) y_advance: Vec2,
    pub(crate) sampler: T,
}

impl<T: Sampler> Shader<T> {
    pub(crate) fn new(transform: Affine, sampler: T) -> Shader<T> {
        let transform = transform * Affine::translate((0.5, 0.5));
        let (x_advance, y_advance) = x_y_advances(&transform);

        Shader {
            transform,
            x_advance,
            y_advance,
            sampler,
        }
    }

    pub fn sample(&self, pos: Point) -> [f32; 4] {
        self.sampler.sample(pos)
    }
}

pub(crate) trait EncodeExt {
    fn encode_into(&self, paints: &mut Vec<EncodedPaint>, transform: Affine) -> Paint;
}

#[derive(Debug)]
pub enum EncodedPaint {
    Image(Shader<EncodedImage>),
    Mask(Shader<EncodedImage>),
    Shading(Shader<EncodedShading>),
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

#[derive(Debug)]
pub struct Buffer<const C: usize> {
    buffer: Vec<f32>,
    width: u32,
    height: u32,
}

impl<const C: usize> Buffer<C> {
    pub fn new_u8(buffer: Vec<u8>, width: u32, height: u32) -> Buffer<C> {
        let converted = buffer.iter().map(|v| *v as f32 * (1.0 / 255.0)).collect();
        Self::new_f32(converted, width, height)
    }

    pub fn new_f32(buffer: Vec<f32>, width: u32, height: u32) -> Buffer<C> {
        assert_eq!(buffer.len() as u32, width * height * C as u32);

        Self {
            buffer,
            width,
            height,
        }
    }

    pub fn sample(&self, pos: Point) -> [f32; C] {
        let x = pos.x as u32;
        let y = pos.y as u32;

        if x >= self.width || y >= self.height {
            [0.0; C]
        } else {
            self.buffer[C * (x + y * self.width) as usize..][..C]
                .try_into()
                .unwrap()
        }
    }
}

impl Buffer<4> {
    pub fn premultiply(&mut self) {
        for ch in self.buffer.chunks_exact_mut(4) {
            let alpha = ch[3];
            ch[0] *= alpha;
            ch[1] *= alpha;
            ch[2] *= alpha;
        }
    }
}
