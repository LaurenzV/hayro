//! Encoding shading patterns for easy sampling.

use crate::canvas::Canvas;
use crate::color::{AlphaColor, ColorSpace};
use crate::function::MAX_COMPONENTS;
use crate::function_shading::draw_function;
use crate::gradient::function_lut;
use crate::interpret::state::ActiveTransferFunction;
use crate::mesh::draw_mesh;
use crate::pattern::ShadingPattern;
use crate::shading::{ShadingFunction, ShadingType};
use crate::util::x_y_advances;
use kurbo::{Affine, Point, Rect};
use std::sync::Arc;

/// A shading pattern that was encoded so it can be sampled.
#[derive(Clone, Debug)]
pub struct EncodedShadingPattern {
    /// The base transform of the shading pattern.
    pub base_transform: Affine,
    pub(crate) color_space: ColorSpace,
    pub(crate) background_color: AlphaColor,
    /// The encoded shading type.
    pub shading_type: EncodedShadingType,
    pub(crate) opacity: f32,
    pub(crate) transfer_function: Option<ActiveTransferFunction>,
}

/// The encoded shading type.
#[derive(Clone, Debug)]
pub enum EncodedShadingType {
    /// Function-based shading.
    FunctionBased(EncodedFunctionBasedShading),
    /// Axial or radial shading.
    RadialAxial(EncodedRadialAxialShading),
    /// Triangle-mesh or patch-mesh shading.
    Mesh(EncodedMeshShading),
    /// Empty dummy shading.
    Dummy(EncodedDummyShading),
}

/// Encoded function-based shading.
#[derive(Clone, Debug)]
pub struct EncodedFunctionBasedShading {
    pub(crate) domain: Rect,
    pub(crate) function: ShadingFunction,
}

/// Encoded axial or radial shading.
#[derive(Clone, Debug)]
pub struct EncodedRadialAxialShading {
    pub(crate) function: ShadingFunction,
    pub(crate) params: RadialAxialParams,
    pub(crate) domain: [f32; 2],
    pub(crate) extend: [bool; 2],
}

/// Encoded triangle-mesh or patch-mesh shading.
#[derive(Clone, Debug)]
pub struct EncodedMeshShading {
    pub(crate) shading_type: Arc<ShadingType>,
}

/// Encoded dummy shading.
#[derive(Clone, Debug)]
pub struct EncodedDummyShading;

/// A shading that was rasterized into a texture.
pub struct ShadingTexture {
    /// The straight (non-premultiplied) RGBA8 pixel data, row-major.
    pub data: Vec<u8>,
    /// The width of the texture in pixels.
    pub width: u32,
    /// The height of the texture in pixels.
    pub height: u32,
    /// The transform from texture space to the space of the bounding box.
    pub transform: Affine,
    /// Whether any pixel of the texture is not fully opaque.
    pub has_transparency: bool,
}

/// The texture dimensions for `bbox` at `scale` pixels per unit.
pub fn texture_dimensions(bbox: Rect, scale: f32) -> (u32, u32) {
    (
        (bbox.width() as f32 * scale).max(1.0).ceil() as u32,
        (bbox.height() as f32 * scale).max(1.0).ceil() as u32,
    )
}

impl EncodedShadingPattern {
    pub(crate) fn finish(&self, mut color: [f32; 4]) -> [f32; 4] {
        color[3] *= self.opacity;

        match &self.transfer_function {
            Some(tf) => tf.apply(&AlphaColor::new(color)).components(),
            None => color,
        }
    }

    pub(crate) fn convert(&self, components: &[f32]) -> [f32; 4] {
        self.finish(self.color_space.to_rgba(components, 1.0).components())
    }

    pub(crate) fn eval(&self, function: &ShadingFunction, input: &[f32]) -> [f32; 4] {
        let mut out = [0.0; MAX_COMPONENTS];
        let out = &mut out[..function.num_outputs()];

        match function.eval(input, out) {
            Some(()) => self.convert(out),
            None => [0.0; 4],
        }
    }

    /// Call `f` with the straight RGBA color of each pixel of a grid covering `bbox`, in row-major order.
    pub fn sample_texture(
        &self,
        bbox: Rect,
        scale: f32,
        f: impl FnMut([f32; 4]),
    ) -> (u32, u32, Affine) {
        let (width, height) = texture_dimensions(bbox, scale);
        let grid_transform =
            Affine::translate((bbox.x0, bbox.y0)) * Affine::scale(1.0 / scale as f64);
        let background = self.finish(self.background_color.components());
        let mut canvas = Canvas::new(width, height, background);
        let transform = grid_transform.inverse() * self.base_transform.inverse();

        match &self.shading_type {
            EncodedShadingType::FunctionBased(shading) => {
                draw_function(
                    &mut canvas,
                    shading.domain,
                    &shading.function,
                    transform,
                    self,
                );
            }
            EncodedShadingType::RadialAxial(shading) => shading.draw(&mut canvas, transform, self),
            EncodedShadingType::Mesh(shading) => {
                draw_mesh(&mut canvas, &shading.shading_type, transform, self);
            }
            EncodedShadingType::Dummy(_) => {}
        }

        canvas.pixels.into_iter().for_each(f);

        (width, height, grid_transform)
    }

    /// Rasterize the shading into a straight RGBA8 texture covering `bbox`.
    pub fn render_texture(&self, bbox: Rect, scale: f32) -> ShadingTexture {
        let (width, height) = texture_dimensions(bbox, scale);
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        let mut has_transparency = false;

        let (_, _, transform) = self.sample_texture(bbox, scale, |sample| {
            let converted = [
                (sample[0] * 255.0 + 0.5) as u8,
                (sample[1] * 255.0 + 0.5) as u8,
                (sample[2] * 255.0 + 0.5) as u8,
                (sample[3] * 255.0 + 0.5) as u8,
            ];

            has_transparency |= converted[3] != 255;
            data.extend_from_slice(&converted);
        });

        ShadingTexture {
            data,
            width,
            height,
            transform,
            has_transparency,
        }
    }
}

impl ShadingPattern {
    /// Encode the shading pattern.
    pub fn encode(&self) -> EncodedShadingPattern {
        let base_transform;

        let shading_type = match self.shading.shading_type.as_ref() {
            ShadingType::FunctionBased {
                domain,
                matrix,
                function,
            } => {
                base_transform = (self.matrix * *matrix).inverse();
                encode_function_shading(domain, function)
            }
            ShadingType::RadialAxial {
                coords,
                domain,
                function,
                extend,
                axial,
            } => {
                let (encoded, initial_transform) =
                    encode_axial_shading(*coords, *domain, function, *extend, *axial);

                base_transform = initial_transform * self.matrix.inverse();

                encoded
            }
            ShadingType::TriangleMesh { .. } | ShadingType::PatchMesh { .. } => {
                base_transform = self.matrix.inverse();

                EncodedShadingType::Mesh(EncodedMeshShading {
                    shading_type: self.shading.shading_type.clone(),
                })
            }
            ShadingType::Dummy => {
                base_transform = Affine::IDENTITY;

                EncodedShadingType::Dummy(EncodedDummyShading)
            }
        };

        let color_space = self.shading.color_space.clone();

        let background_color = match (&shading_type, &self.shading.background) {
            (EncodedShadingType::Dummy(_), _) | (_, None) => AlphaColor::TRANSPARENT,
            (_, Some(background)) => color_space.to_rgba(background, 1.0),
        };

        EncodedShadingPattern {
            color_space,
            background_color,
            shading_type,
            base_transform,
            opacity: self.opacity,
            transfer_function: self.transfer_function.clone(),
        }
    }
}

fn encode_axial_shading(
    coords: [f32; 6],
    domain: [f32; 2],
    function: &ShadingFunction,
    extend: [bool; 2],
    is_axial: bool,
) -> (EncodedShadingType, Affine) {
    let initial_transform;

    let coords = coords.map(f64::from);
    let params = if is_axial {
        let [x_0, y_0, x_1, y_1, _, _] = coords;

        initial_transform = ts_from_line_to_line(
            Point::new(x_0, y_0),
            Point::new(x_1, y_1),
            Point::ZERO,
            Point::new(1.0, 0.0),
        );

        RadialAxialParams::Axial
    } else {
        let [x_0, y_0, r0, x_1, y_1, r_1] = coords;

        initial_transform = Affine::translate((-x_0, -y_0));
        let new_x1 = x_1 - x_0;
        let new_y1 = y_1 - y_0;

        let p1 = Point::new(new_x1, new_y1);
        let r = Point::new(r0, r_1);

        RadialAxialParams::Radial { p1, r }
    };

    (
        EncodedShadingType::RadialAxial(EncodedRadialAxialShading {
            function: function.clone(),
            params,
            domain,
            extend,
        }),
        initial_transform,
    )
}

fn encode_function_shading(domain: &[f32; 4], function: &ShadingFunction) -> EncodedShadingType {
    let domain = Rect::new(
        domain[0] as f64,
        domain[2] as f64,
        domain[1] as f64,
        domain[3] as f64,
    );

    EncodedShadingType::FunctionBased(EncodedFunctionBasedShading {
        domain,
        function: function.clone(),
    })
}

#[derive(Clone, Debug)]
pub(crate) enum RadialAxialParams {
    Axial,
    Radial { p1: Point, r: Point },
}

impl EncodedRadialAxialShading {
    fn draw(&self, canvas: &mut Canvas, transform: Affine, pattern: &EncodedShadingPattern) {
        let lut = function_lut(&self.function, self.domain, pattern);
        let to_shading = transform.inverse();
        let (x_advance, y_advance) = x_y_advances(&to_shading);
        let mut row_start = to_shading * Point::new(0.5, 0.5);

        for row in canvas.pixels.chunks_exact_mut(canvas.width) {
            let mut point = row_start;

            for pixel in row {
                if let Some(t) = self.position(point) {
                    *pixel = lut.get(t);
                }

                point += x_advance;
            }

            row_start += y_advance;
        }
    }

    fn position(&self, pos: Point) -> Option<f32> {
        let t = match self.params {
            RadialAxialParams::Axial => pos.x as f32,
            RadialAxialParams::Radial { p1, r } => {
                return radial_pos(pos, p1, r, self.extend).map(|t| t.clamp(0.0, 1.0) as f32);
            }
        };

        if t < 0.0 {
            self.extend[0].then_some(0.0)
        } else if t > 1.0 {
            self.extend[1].then_some(1.0)
        } else {
            Some(t)
        }
    }
}

fn ts_from_line_to_line(src1: Point, src2: Point, dst1: Point, dst2: Point) -> Affine {
    let unit_to_line1 = unit_to_line(src1, src2);
    let line1_to_unit = unit_to_line1.inverse();
    let unit_to_line2 = unit_to_line(dst1, dst2);

    unit_to_line2 * line1_to_unit
}

fn unit_to_line(p0: Point, p1: Point) -> Affine {
    Affine::new([
        p1.y - p0.y,
        p0.x - p1.x,
        p1.x - p0.x,
        p1.y - p0.y,
        p0.x,
        p0.y,
    ])
}

fn radial_pos(pos: Point, p1: Point, r: Point, extend: [bool; 2]) -> Option<f64> {
    if r.x == 0.0 && r.y == 0.0 {
        return None;
    }
    let dr = r.y - r.x;
    let r0 = r.x;
    let a = norm_squared_minus_square(p1, dr);
    let bx = pos.x * p1.x;
    let by = pos.y * p1.y;
    let br = r0 * dr;
    // Add the smallest product last so cancellation can expose it.
    let b = if bx.abs() <= by.abs() && bx.abs() <= br.abs() {
        (by + br) + bx
    } else if by.abs() <= br.abs() {
        (bx + br) + by
    } else {
        (bx + by) + br
    };
    let c = || norm_squared_minus_square(pos, r.x);
    let valid = |t: f64| (extend[0] || t >= 0.0) && (extend[1] || t <= 1.0) && r.x + dr * t >= 0.0;

    if a == 0.0 {
        if b == 0.0 {
            return [1.0, 0.0].into_iter().find(|&t| c() == 0.0 && valid(t));
        }
        let t = c() / (2.0 * b);
        return valid(t).then_some(t);
    }

    let (dx, dy) = (p1.x, p1.y);
    let (x, y) = (pos.x, pos.y);
    let u = r0 * dx + dr * x;
    let v = r0 * dy + dr * y;
    // Factor u² + v² − (x*dy − y*dx)² before the cross product loses its smaller term.
    let d = if u.abs() >= v.abs() {
        (dx * (r0 - y) + x * (dr + dy)) * (dx * (r0 + y) + x * (dr - dy)) + v * v
    } else {
        (dy * (r0 + x) + y * (dr - dx)) * (dy * (r0 - x) + y * (dr + dx)) + u * u
    };
    if d < 0.0 {
        return None;
    }
    let sqrt_d = d.sqrt();

    // Add terms of the same sign, then recover the other root from their product c/a.
    let q = b + sqrt_d.copysign(b);

    if q == 0.0 {
        return (c() == 0.0 && valid(0.0)).then_some(0.0);
    }

    let t = q / a;
    let other = c() / q;
    let scale = dx.abs().max(dy.abs()).max(dr.abs());
    // PDF generators round focal-on-circle coordinates to ~6 significant digits
    // (a / scale² ≤ 2e-6), turning a single-root tangent cone into a distant bogus root.
    if a > 0.0 && dr > 0.0 && extend[1] && (a / scale) / scale <= 2e-6 && valid(other) {
        return Some(other);
    }

    let (max, min) = if q.is_sign_negative() == a.is_sign_negative() {
        (t, other)
    } else {
        (other, t)
    };
    if valid(max) {
        Some(max)
    } else {
        valid(min).then_some(min)
    }
}

fn norm_squared_minus_square(p: Point, r: f64) -> f64 {
    let (major, minor) = if p.x.abs() >= p.y.abs() {
        (p.x.abs(), p.y)
    } else {
        (p.y.abs(), p.x)
    };
    (major - r.abs()) * (major + r.abs()) + minor * minor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use hayro_syntax::object::{Dict, FromBytes};

    #[test]
    fn zero_radius_pattern_and_shading_operator() {
        use crate::font::GlyphRun;
        use crate::pattern::Pattern;
        use crate::{
            BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
            InterpreterCache, Paint, SoftMask, interpret_page,
        };
        use hayro_syntax::Pdf;
        use kurbo::BezPath;

        struct Samples(Vec<Vec<u8>>);
        impl Device<'_> for Samples {
            fn draw_path(&mut self, _: &BezPath, props: DrawProps<'_>, _: &DrawMode) {
                let Paint::Pattern(pattern) = props.paint else {
                    panic!("expected pattern paint")
                };
                let Pattern::Shading(pattern) = *pattern else {
                    panic!("expected shading pattern")
                };
                let encoded = pattern.encode();
                let EncodedShadingType::RadialAxial(gradient) = &encoded.shading_type else {
                    panic!("expected radial shading");
                };
                assert!(gradient.as_svg_gradient(&encoded, 0.01).is_none());
                self.0.push(
                    encoded
                        .render_texture(Rect::new(0.0, 0.0, 1.0, 1.0), 1.0)
                        .data,
                );
            }
            fn push_clip_path(&mut self, _: &ClipPath) {}
            fn push_transparency_group(
                &mut self,
                _: f32,
                _: Option<SoftMask<'_>>,
                _: BlendMode,
                _: bool,
                _: bool,
            ) {
            }
            fn draw_glyph_run(&mut self, _: &GlyphRun<'_, '_>, _: DrawProps<'_>, _: &DrawMode) {}
            fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}
            fn pop_clip(&mut self) {}
            fn pop_transparency_group(&mut self) {}
        }

        let ops = "/Pattern cs /P scn 0 0 1 1 re f /S sh";
        let pdf = format!("%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 4 0 R
/Resources << /Pattern << /P << /PatternType 2 /Shading 5 0 R >> >> /Shading << /S 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{ops}
endstream endobj
5 0 obj << /ShadingType 3 /ColorSpace /DeviceRGB /Background [1 0 0]
/Coords [0 0.5 0 1 0.5 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1 >> >> endobj
trailer << /Root 1 0 R >>
%%EOF", ops.len());
        let pdf = Pdf::new(pdf.into_bytes()).unwrap();
        let mut context = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            &InterpreterCache::new(),
            pdf.xref(),
            crate::InterpreterSettings::default(),
        );
        let mut samples = Samples(Vec::new());
        interpret_page(&pdf.pages()[0], &mut context, &mut samples);
        assert_eq!(samples.0, [[255, 0, 0, 255], [0, 0, 0, 0]]);
        assert_eq!(
            radial_pos(
                Point::new(0.5, 0.0),
                Point::new(1.0, 0.0),
                Point::ZERO,
                [false; 2]
            ),
            None
        );
    }

    #[test]
    fn rounded_radial_focal_points() {
        for (x, y) in [
            (0.223151, 0.974784),
            (0.28201, 0.959412),
            (0.999999, 0.00148365),
        ] {
            for scale in [0.001, 1.0, 1000.0] {
                let axis = Point::new(x * scale, y * scale);
                for t in [0.5, 1.5] {
                    let pos = Point::new((x + 1.0) * scale * t, y * scale * t);
                    let forward = radial_pos(pos, axis, Point::new(0.0, scale), [true; 2]);
                    let backward = radial_pos(
                        Point::new(pos.x - axis.x, pos.y - axis.y),
                        Point::new(-axis.x, -axis.y),
                        Point::new(scale, 0.0),
                        [true, false],
                    );
                    assert!((forward.unwrap() - t).abs() < 1e-12);
                    assert!((backward.unwrap() - (1.0 - t)).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn nearly_tangent_radial_extensions() {
        for (dx, r0, r1, x, extend, expected) in [
            (
                10000.05_f32,
                0.0,
                10000.0,
                0.0605,
                [false, true],
                1.214745098,
            ),
            (131073.0, 0.0, 131072.0, 32768.0, [false, true], 32768.0),
            (131073.0, 0.0, 131072.0, 32769.0, [false, true], 32769.0),
            (1048577.0, 1.0, 1048577.0, 0.5, [false, true], 1.5),
            (99.0005, 100.0, 1.0, 98.75, [true, false], -2482.424242),
            (100.0005, 0.0, 100.0, 5.75, [false, true], 11419.151515),
        ] {
            let t = radial_pos(
                Point::new(x, 0.0),
                Point::new(f64::from(dx), 0.0),
                Point::new(r0, r1),
                extend,
            )
            .unwrap();
            assert!((t - expected).abs() < 1e-6, "{t} != {expected}");
        }
    }

    #[test]
    fn later_extended_radial_root() {
        for (x, dx, r0, expected) in [(0.75, 1.25, 0.0, 3.0), (1.5, 1.0, 1.0, 2.5)] {
            assert_eq!(
                radial_pos(
                    Point::new(x, 0.0),
                    Point::new(dx, 0.0),
                    Point::new(r0, 1.0),
                    [false, true]
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn radial_precision() {
        let sample = |coords: [f32; 6], pos: Point, extend: [bool; 2]| {
            let [x0, y0, r0, x1, y1, r1] = coords;
            let [start, end] = extend;
            let data = format!(
                "<< /Shading << /ShadingType 3 /ColorSpace /DeviceGray /Coords [{x0} {y0} {r0} {x1} {y1} {r1}] /Extend [{start} {end}] /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >> >>"
            );
            let dict = Dict::from_bytes(data.as_bytes()).unwrap();
            let pattern = ShadingPattern::new(&dict, &Cache::new()).unwrap().encode();
            match pattern.shading_type {
                EncodedShadingType::Dummy(_) => None,
                EncodedShadingType::RadialAxial(shading) => {
                    shading.position(pattern.base_transform * pos)
                }
                _ => unreachable!(),
            }
        };
        for (coords, x, y, expected) in [
            ([0.0, 0.5, 0.0, 8193.0, 0.5, 0.0], 0.5, 0.5, None),
            (
                [-1073741824.0, 0.0, 1.0, 1073741824.0, 0.0, 1.0],
                0.5,
                2.5,
                None,
            ),
            ([0.0, 0.0, 4.0, 3.0, 4.0, 7.0], 4.0, -3.0, Some(0.75)),
            (
                [0.0, 0.0, 0.0, 134217728.0, 1.0, 134217728.0],
                0.0,
                0.75,
                Some(0.75),
            ),
            (
                [0.0, 0.0, 0.0, 134217728.0, 1.0, 134217728.0],
                2_f64.powi(-31),
                0.5,
                Some(0.8201941),
            ),
            ([0.0, 0.0, 3.0, 1.0, 1.0, 3.0], -2.0, 1.0, Some(1.0)),
            (
                [0.0, 0.0, 3.0, 1.0, 1.0, 3.0],
                -2.001,
                1.0,
                Some(0.99899983),
            ),
            ([0.0, 0.0, 3.0, 1.0, 1.0, 3.0], -1.999, 1.0, None),
            (
                [0.0, 0.0, 2.5, 64.1, 0.0, 2.5],
                0.5,
                2.5,
                Some((0.5 / f64::from(64.1_f32)) as f32),
            ),
            ([0.0, 0.0, 2.5, 64.1, 0.0, 2.5], 0.5, 2.501, None),
            ([0.0, 0.0, 1.0, 1.0, 0.0, 2.0], -1.0, 0.0, Some(1.0)),
        ] {
            let [x0, y0, r0, x1, y1, r1] = coords;
            assert_eq!(sample(coords, Point::new(x, y), [false; 2]), expected);
            assert_eq!(
                sample([y0, x0, r0, y1, x1, r1], Point::new(y, x), [false; 2]),
                expected
            );
        }
        for extend in [[true, false], [false, true], [true; 2]] {
            assert_eq!(
                sample(
                    [0.0, 0.0, 1.0, 1.0, 0.0, 2.0],
                    Point::new(-1.0, 0.0),
                    extend
                ),
                Some(1.0)
            );
        }
        for extend in [[false; 2], [true, false], [false, true], [true; 2]] {
            let l = 134217728.0;
            for (y, expected) in [(0.5, Some(0.5)), (-0.5, extend[0].then_some(0.0))] {
                assert_eq!(
                    sample([0.0, 0.0, l, l, 1.0, 0.0], Point::new(l.into(), y), extend),
                    expected
                );
                assert_eq!(
                    sample([0.0, 0.0, l, 1.0, l, 0.0], Point::new(y, l.into()), extend),
                    expected
                );
            }
            let coords = [0.0, 0.0, 3.0, 0.0, 0.0, 1.0];
            for (x, expected) in [
                (4.0, extend[0].then_some(0.0)),
                (0.5, extend[1].then_some(1.0)),
                (2.0, Some(0.5)),
            ] {
                assert_eq!(sample(coords, Point::new(x, 0.0), extend), expected);
            }
        }
        for scale in [1e-30, 1e30] {
            let t = radial_pos(
                Point::new(scale, 0.0),
                Point::new(2.0 * scale, 0.0),
                Point::new(scale / 2.0, scale / 2.0),
                [false; 2],
            )
            .unwrap();
            assert!((t - 0.75).abs() < 1e-14);
        }
    }
}
