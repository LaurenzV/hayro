// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::coarse::Wide;
use crate::encode::{EncodeExt, EncodedPaint};
use crate::fine::Fine;
use crate::flatten::Line;
use crate::mask::Mask;
use crate::paint::{Paint, PaintType};
use crate::pixmap::Pixmap;
use crate::strip::Strip;
use crate::tile::Tiles;
use crate::{flatten, strip};
use hayro_interpret::FillRule;
use kurbo::{Affine, BezPath, Cap, Join, Rect, Shape, Stroke};
use std::vec;
use std::vec::Vec;

pub(crate) const DEFAULT_TOLERANCE: f64 = 0.1;
/// A render context.
#[derive(Debug)]
pub(crate) struct RenderContext {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) wide: Wide,
    pub(crate) alphas: Vec<u8>,
    pub(crate) line_buf: Vec<Line>,
    pub(crate) tiles: Tiles,
    pub(crate) strip_buf: Vec<Strip>,
    paint_clip_path: Option<BezPath>,
    pub(crate) stroke: Stroke,
    pub(crate) transform: Affine,
    pub(crate) fill_rule: FillRule,
    pub(crate) encoded_paints: Vec<EncodedPaint>,
    pub(crate) anti_aliasing: bool,
}

impl RenderContext {
    /// Create a new render context with the given width and height in pixels.
    pub(crate) fn new(width: u16, height: u16) -> Self {
        let wide = Wide::new(width, height);

        let alphas = vec![];
        let line_buf = vec![];
        let tiles = Tiles::new();
        let strip_buf = vec![];

        let transform = Affine::IDENTITY;
        let fill_rule = FillRule::NonZero;
        let stroke = Stroke {
            width: 1.0,
            join: Join::Bevel,
            start_cap: Cap::Butt,
            end_cap: Cap::Butt,
            ..Default::default()
        };
        let encoded_paints = vec![];
        let anti_aliasing = true;

        Self {
            width,
            height,
            wide,
            alphas,
            line_buf,
            tiles,
            strip_buf,
            transform,
            paint_clip_path: None,
            fill_rule,
            stroke,
            encoded_paints,
            anti_aliasing,
        }
    }

    fn encode_paint(&mut self, paint_type: PaintType) -> Paint {
        match paint_type {
            PaintType::Solid(s) => {
                self.paint_clip_path = None;
                s.into()
            }
            PaintType::Image(i) => {
                self.paint_clip_path = None;
                i.encode_into(&mut self.encoded_paints)
            }
            PaintType::ShadingPattern(s) => {
                self.paint_clip_path = s.shading.clip_path.clone();
                s.encode_into(&mut self.encoded_paints)
            }
        }
    }

    /// Fill a path.
    pub(crate) fn fill_path(&mut self, path: &BezPath, paint_type: PaintType, mask: Option<Mask>) {
        let paint = self.encode_paint(paint_type);
        self.apply_paint_bbox();
        flatten::fill(path, self.transform, &mut self.line_buf);
        self.render_path(self.fill_rule, paint, mask);
        self.unapply_paint_bbox();
    }

    fn apply_paint_bbox(&mut self) {
        if let Some(clip_path) = self.paint_clip_path.clone() {
            self.push_layer(Some(&clip_path), None, None);
        }
    }

    fn unapply_paint_bbox(&mut self) {
        if self.paint_clip_path.is_some() {
            self.pop_layer();
        }
    }

    /// Stroke a path.
    pub(crate) fn stroke_path(
        &mut self,
        path: &BezPath,
        paint_type: PaintType,
        mask: Option<Mask>,
    ) {
        let paint = self.encode_paint(paint_type);
        self.apply_paint_bbox();
        flatten::stroke(path, &self.stroke, self.transform, &mut self.line_buf);
        self.render_path(FillRule::NonZero, paint, mask);
        self.unapply_paint_bbox();
    }

    /// Fill a rectangle.
    pub(crate) fn fill_rect(&mut self, rect: &Rect, paint_type: PaintType, mask: Option<Mask>) {
        self.fill_path(&rect.to_path(DEFAULT_TOLERANCE), paint_type, mask);
    }

    /// Push a new layer with the given properties.
    ///
    /// Note that the mask, if provided, needs to have the same size as the render context. Otherwise,
    /// it will be ignored. In addition to that, the mask will not be affected by the current
    /// transformation matrix in place.
    pub(crate) fn push_layer(
        &mut self,
        clip_path: Option<&BezPath>,
        opacity: Option<f32>,
        mask: Option<Mask>,
    ) {
        let clip = if let Some(c) = clip_path {
            flatten::fill(c, Affine::IDENTITY, &mut self.line_buf);
            self.make_strips(self.fill_rule);
            Some((self.strip_buf.as_slice(), self.fill_rule))
        } else {
            None
        };

        let mask = mask.and_then(|m| {
            if m.width() != self.width || m.height() != self.height {
                None
            } else {
                Some(m)
            }
        });

        self.wide.push_layer(clip, mask, opacity.unwrap_or(1.0));
    }

    /// Pop the last-pushed layer.
    pub(crate) fn pop_layer(&mut self) {
        self.wide.pop_layer();
    }

    /// Set the current stroke.
    pub(crate) fn set_stroke(&mut self, stroke: Stroke) {
        self.stroke = stroke;
    }

    /// Set the current fill rule.
    pub(crate) fn set_fill_rule(&mut self, fill_rule: FillRule) {
        self.fill_rule = fill_rule;
    }

    /// Set the current transform.
    pub(crate) fn set_transform(&mut self, transform: Affine) {
        self.transform = transform;
    }

    /// Render the current context into a buffer.
    /// The buffer is expected to be in premultiplied RGBA8 format with length `width * height * 4`
    pub(crate) fn render_to_buffer(&self, buffer: &mut [u8], width: u16, height: u16) {
        assert!(
            !self.wide.has_layers(),
            "some layers haven't been popped yet"
        );
        assert_eq!(
            buffer.len(),
            (width as usize) * (height as usize) * 4,
            "provided width ({}) and height ({}) do not match buffer size ({})",
            width,
            height,
            buffer.len(),
        );

        let mut fine = Fine::new(width, height);
        let width_tiles = self.wide.width_tiles();
        let height_tiles = self.wide.height_tiles();
        for y in 0..height_tiles {
            for x in 0..width_tiles {
                let wtile = self.wide.get(x, y);
                fine.set_coords(x, y);

                fine.clear(wtile.bg.0);
                for cmd in &wtile.cmds {
                    fine.run_cmd(cmd, &self.alphas, &self.encoded_paints);
                }
                fine.pack(buffer);
            }
        }
    }

    /// Render the current context into a pixmap.
    pub(crate) fn render_to_pixmap(&self, pixmap: &mut Pixmap) {
        let width = pixmap.width();
        let height = pixmap.height();
        self.render_to_buffer(pixmap.data_as_u8_slice_mut(), width, height);
    }

    pub(crate) fn set_anti_aliasing(&mut self, val: bool) {
        self.anti_aliasing = val;
    }

    // Assumes that `line_buf` contains the flattened path.
    fn render_path(&mut self, fill_rule: FillRule, paint: Paint, mask: Option<Mask>) {
        self.make_strips(fill_rule);
        self.wide.generate(&self.strip_buf, fill_rule, paint, mask);
    }

    fn make_strips(&mut self, fill_rule: FillRule) {
        self.tiles
            .make_tiles(&self.line_buf, self.width, self.height);
        self.tiles.sort_tiles();

        strip::render(
            &self.tiles,
            &mut self.strip_buf,
            &mut self.alphas,
            fill_rule,
            &self.line_buf,
            self.anti_aliasing,
        );
    }
}
