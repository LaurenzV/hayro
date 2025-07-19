// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::encode::Shader;
use crate::fine::{COLOR_COMPONENTS, Painter, Sampler, TILE_HEIGHT_COMPONENTS};
use crate::paint::PremulColor;
use kurbo::Point;

#[derive(Debug)]
pub(crate) struct ShaderFiller<'a, T: Sampler> {
    cur_pos: Point,
    shader: &'a Shader<T>,
}

impl<'a, T: Sampler> ShaderFiller<'a, T> {
    pub(crate) fn new(shader: &'a Shader<T>, start_x: u16, start_y: u16) -> Self {
        Self {
            cur_pos: shader.transform * Point::new(f64::from(start_x), f64::from(start_y)),
            shader,
        }
    }

    pub(super) fn run(mut self, target: &mut [f32]) {
        target
            .chunks_exact_mut(TILE_HEIGHT_COMPONENTS)
            .for_each(|column| {
                let mut pos = self.cur_pos;

                for pixel in column.chunks_exact_mut(COLOR_COMPONENTS) {
                    let color = self.shader.sample(pos);
                    pixel.copy_from_slice(&PremulColor(color).0);

                    pos += self.shader.y_advance;
                }

                self.cur_pos += self.shader.x_advance;
            });
    }
}

impl<T: Sampler> Painter for ShaderFiller<'_, T> {
    fn paint(self, target: &mut [f32]) {
        self.run(target);
    }
}
