//! PDF colors and color spaces.

use crate::cache::Cache;
use hayro_syntax::function::Function;
use hayro_syntax::object;
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use log::warn;
use qcms::Transform;
use smallvec::{SmallVec, ToSmallVec, smallvec};
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::{Arc, LazyLock};

/// A storage for the components of colors.
pub type ColorComponents = SmallVec<[f32; 4]>;

/// An RGB color with an alpha channel.
#[derive(Debug, Copy, Clone)]
pub struct AlphaColor {
    components: [f32; 4],
}

impl AlphaColor {
    /// A black color.
    pub const BLACK: Self = Self::new([0., 0., 0., 1.]);

    /// A transparent color.
    pub const TRANSPARENT: Self = Self::new([0., 0., 0., 0.]);

    /// A white color.
    pub const WHITE: Self = Self::new([1., 1., 1., 1.]);

    /// Create a new color from the given components.
    pub const fn new(components: [f32; 4]) -> Self {
        Self { components }
    }

    /// Create a new color from RGB8 values.
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        let components = [u8_to_f32(r), u8_to_f32(g), u8_to_f32(b), 1.];
        Self::new(components)
    }

    /// Return the color as premulitplied RGBF32.
    pub fn premultiplied(&self) -> [f32; 4] {
        [
            self.components[0] * self.components[3],
            self.components[1] * self.components[3],
            self.components[2] * self.components[3],
            self.components[3],
        ]
    }

    /// Create a new color from RGBA8 values.
    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let components = [u8_to_f32(r), u8_to_f32(g), u8_to_f32(b), u8_to_f32(a)];
        Self::new(components)
    }

    /// Return the color as RGBA8.
    pub fn to_rgba8(&self) -> [u8; 4] {
        [
            (self.components[0] * 255.0 + 0.5) as u8,
            (self.components[1] * 255.0 + 0.5) as u8,
            (self.components[2] * 255.0 + 0.5) as u8,
            (self.components[3] * 255.0 + 0.5) as u8,
        ]
    }

    /// Return the components of the color as RGBF32.
    pub fn components(&self) -> [f32; 4] {
        self.components
    }
}

const fn u8_to_f32(x: u8) -> f32 {
    x as f32 * (1.0 / 255.0)
}

#[derive(Debug, Clone)]
enum ColorSpaceType {
    DeviceCmyk,
    DeviceGray,
    DeviceRgb,
    Pattern(ColorSpace),
    Indexed(Indexed),
    ICCBased(ICCProfile),
    CalGray(CalGray),
    CalRgb(CalRgb),
    Lab(Lab),
    Separation(Separation),
    DeviceN(DeviceN),
}

impl ColorSpaceType {
    fn new(object: Object, cache: &Cache) -> Option<Self> {
        Self::new_inner(object, cache)
    }

    fn new_inner(object: Object, cache: &Cache) -> Option<ColorSpaceType> {
        if let Some(name) = object.clone().into_name() {
            return Self::new_from_name(name.clone());
        } else if let Some(color_array) = object.clone().into_array() {
            let mut iter = color_array.clone().flex_iter();
            let name = iter.next::<Name>()?;

            match name.deref() {
                ICC_BASED => {
                    let icc_stream = iter.next::<Stream>()?;
                    let dict = icc_stream.dict();
                    let num_components = dict.get::<usize>(N)?;

                    return cache.get_or_insert_with(icc_stream.obj_id(), || {
                        if let Some(decoded) = icc_stream.decoded().ok().as_ref() {
                            ICCProfile::new(decoded, num_components)
                                .map(ColorSpaceType::ICCBased)
                                .or_else(|| {
                                    dict.get::<Object>(ALTERNATE)
                                        .and_then(|o| ColorSpaceType::new(o, cache))
                                })
                                .or_else(|| match dict.get::<u8>(N) {
                                    Some(1) => Some(ColorSpaceType::DeviceGray),
                                    Some(3) => Some(ColorSpaceType::DeviceRgb),
                                    Some(4) => Some(ColorSpaceType::DeviceCmyk),
                                    _ => None,
                                })
                        } else {
                            None
                        }
                    });
                }
                CALCMYK => return Some(ColorSpaceType::DeviceCmyk),
                CALGRAY => {
                    let cal_dict = iter.next::<Dict>()?;
                    return Some(ColorSpaceType::CalGray(CalGray::new(&cal_dict)?));
                }
                CALRGB => {
                    let cal_dict = iter.next::<Dict>()?;
                    return Some(ColorSpaceType::CalRgb(CalRgb::new(&cal_dict)?));
                }
                DEVICE_RGB | RGB => return Some(ColorSpaceType::DeviceRgb),
                DEVICE_GRAY | G => return Some(ColorSpaceType::DeviceGray),
                DEVICE_CMYK | CMYK => return Some(ColorSpaceType::DeviceCmyk),
                LAB => {
                    let lab_dict = iter.next::<Dict>()?;
                    return Some(ColorSpaceType::Lab(Lab::new(&lab_dict)?));
                }
                INDEXED | I => {
                    return Some(ColorSpaceType::Indexed(Indexed::new(&color_array, cache)?));
                }
                SEPARATION => {
                    return Some(ColorSpaceType::Separation(Separation::new(
                        &color_array,
                        cache,
                    )?));
                }
                DEVICE_N => {
                    return Some(ColorSpaceType::DeviceN(DeviceN::new(&color_array, cache)?));
                }
                PATTERN => {
                    let _ = iter.next::<Name>();
                    let cs = iter
                        .next::<Object>()
                        .and_then(|o| ColorSpace::new(o, cache))
                        .unwrap_or(ColorSpace::device_rgb());
                    return Some(ColorSpaceType::Pattern(cs));
                }
                _ => {
                    warn!("unsupported color space: {}", name.as_str());
                    return None;
                }
            }
        }

        None
    }

    fn new_from_name(name: Name) -> Option<Self> {
        match name.deref() {
            DEVICE_RGB | RGB => Some(ColorSpaceType::DeviceRgb),
            DEVICE_GRAY | G => Some(ColorSpaceType::DeviceGray),
            DEVICE_CMYK | CMYK => Some(ColorSpaceType::DeviceCmyk),
            CALCMYK => Some(ColorSpaceType::DeviceCmyk),
            PATTERN => Some(ColorSpaceType::Pattern(ColorSpace::device_rgb())),
            _ => None,
        }
    }
}

/// A PDF color space.
#[derive(Debug, Clone)]
pub struct ColorSpace(Arc<ColorSpaceType>);

impl ColorSpace {
    /// Create a new color space from the given object.
    pub(crate) fn new(object: Object, cache: &Cache) -> Option<ColorSpace> {
        Some(Self(Arc::new(ColorSpaceType::new(object, cache)?)))
    }

    /// Create a new color space from the name.
    pub(crate) fn new_from_name(name: Name) -> Option<ColorSpace> {
        ColorSpaceType::new_from_name(name).map(|c| Self(Arc::new(c)))
    }

    /// Return the device gray color space.
    pub(crate) fn device_gray() -> ColorSpace {
        Self(Arc::new(ColorSpaceType::DeviceGray))
    }

    /// Return the device RGB color space.
    pub(crate) fn device_rgb() -> ColorSpace {
        Self(Arc::new(ColorSpaceType::DeviceRgb))
    }

    /// Return the device CMYK color space.
    pub(crate) fn device_cmyk() -> ColorSpace {
        Self(Arc::new(ColorSpaceType::DeviceCmyk))
    }

    /// Return the pattern color space.
    pub(crate) fn pattern() -> ColorSpace {
        Self(Arc::new(ColorSpaceType::Pattern(ColorSpace::device_gray())))
    }

    pub(crate) fn pattern_cs(&self) -> Option<ColorSpace> {
        match self.0.as_ref() {
            ColorSpaceType::Pattern(cs) => Some(cs.clone()),
            _ => None,
        }
    }

    /// Return `true` if the current color space is the pattern color space.
    pub(crate) fn is_pattern(&self) -> bool {
        matches!(self.0.as_ref(), ColorSpaceType::Pattern(_))
    }

    /// Return `true` if the current color space is an indexed color space.
    pub(crate) fn is_indexed(&self) -> bool {
        matches!(self.0.as_ref(), ColorSpaceType::Indexed(_))
    }

    /// Get the default decode array for the color space.
    pub(crate) fn default_decode_arr(&self, n: f32) -> SmallVec<[(f32, f32); 4]> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk => smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0), (0.0, 1.0)],
            ColorSpaceType::DeviceGray => smallvec![(0.0, 1.0)],
            ColorSpaceType::DeviceRgb => smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0)],
            ColorSpaceType::ICCBased(i) => smallvec![(0.0, 1.0); i.0.number_components],
            ColorSpaceType::CalGray(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::CalRgb(_) => smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0)],
            ColorSpaceType::Lab(l) => smallvec![
                (0.0, 100.0),
                (l.range[0], l.range[1]),
                (l.range[2], l.range[3]),
            ],
            ColorSpaceType::Indexed(_) => smallvec![(0.0, 2.0f32.powf(n) - 1.0)],
            ColorSpaceType::Separation(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::DeviceN(d) => smallvec![(0.0, 1.0); d.num_components],
            // Not a valid image color space.
            ColorSpaceType::Pattern(_) => smallvec![(0.0, 1.0)],
        }
    }

    /// Get the initial color of the color space.
    pub(crate) fn initial_color(&self) -> ColorComponents {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk => smallvec![0.0, 0.0, 0.0, 1.0],
            ColorSpaceType::DeviceGray => smallvec![0.0],
            ColorSpaceType::DeviceRgb => smallvec![0.0, 0.0, 0.0],
            ColorSpaceType::ICCBased(icc) => match icc.0.number_components {
                1 => smallvec![0.0],
                3 => smallvec![0.0, 0.0, 0.0],
                4 => smallvec![0.0, 0.0, 0.0, 1.0],
                _ => unreachable!(),
            },
            ColorSpaceType::CalGray(_) => smallvec![0.0],
            ColorSpaceType::CalRgb(_) => smallvec![0.0, 0.0, 0.0],
            ColorSpaceType::Lab(_) => smallvec![0.0, 0.0, 0.0],
            ColorSpaceType::Indexed(_) => smallvec![0.0],
            ColorSpaceType::Separation(_) => smallvec![1.0],
            ColorSpaceType::Pattern(c) => c.initial_color(),
            ColorSpaceType::DeviceN(d) => smallvec![1.0; d.num_components],
        }
    }

    /// Get the number of components of the color space.
    pub(crate) fn num_components(&self) -> u8 {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk => 4,
            ColorSpaceType::DeviceGray => 1,
            ColorSpaceType::DeviceRgb => 3,
            ColorSpaceType::ICCBased(icc) => icc.0.number_components as u8,
            ColorSpaceType::CalGray(_) => 1,
            ColorSpaceType::CalRgb(_) => 3,
            ColorSpaceType::Lab(_) => 3,
            ColorSpaceType::Indexed(_) => 1,
            ColorSpaceType::Separation(_) => 1,
            ColorSpaceType::Pattern(p) => p.num_components(),
            ColorSpaceType::DeviceN(d) => d.num_components as u8,
        }
    }

    /// Turn the given component values and opacity into an RGBA color.
    pub fn to_rgba(&self, c: &[f32], opacity: f32) -> AlphaColor {
        self.to_rgba_inner(c, opacity).unwrap_or(AlphaColor::BLACK)
    }

    fn to_rgba_inner(&self, c: &[f32], opacity: f32) -> Option<AlphaColor> {
        let color = match self.0.as_ref() {
            ColorSpaceType::DeviceRgb => {
                AlphaColor::new([*c.first()?, *c.get(1)?, *c.get(2)?, opacity])
            }
            ColorSpaceType::DeviceGray => {
                AlphaColor::new([*c.first()?, *c.first()?, *c.first()?, opacity])
            }
            ColorSpaceType::DeviceCmyk => {
                let opacity = f32_to_u8(opacity);
                let srgb = CMYK_TRANSFORM.to_rgb(c)?;

                AlphaColor::from_rgba8(srgb[0], srgb[1], srgb[2], opacity)
            }
            ColorSpaceType::ICCBased(icc) => {
                let opacity = f32_to_u8(opacity);
                let srgb = icc.to_rgb(c)?;

                AlphaColor::from_rgba8(srgb[0], srgb[1], srgb[2], opacity)
            }
            ColorSpaceType::CalGray(cal) => {
                let opacity = f32_to_u8(opacity);
                let srgb = cal.to_rgb(*c.first()?);

                AlphaColor::from_rgba8(srgb[0], srgb[1], srgb[2], opacity)
            }
            ColorSpaceType::CalRgb(cal) => {
                let opacity = f32_to_u8(opacity);
                let srgb = cal.to_rgb([*c.first()?, *c.get(1)?, *c.get(2)?]);

                AlphaColor::from_rgba8(srgb[0], srgb[1], srgb[2], opacity)
            }
            ColorSpaceType::Lab(lab) => {
                let opacity = f32_to_u8(opacity);
                let srgb = lab.to_rgb([*c.first()?, *c.get(1)?, *c.get(2)?]);

                AlphaColor::from_rgba8(srgb[0], srgb[1], srgb[2], opacity)
            }
            ColorSpaceType::Indexed(i) => i.to_rgb(*c.first()?, opacity),
            ColorSpaceType::Separation(s) => s.to_rgba(*c.first()?, opacity),
            ColorSpaceType::Pattern(_) => AlphaColor::BLACK,
            ColorSpaceType::DeviceN(d) => d.to_rgba(c, opacity),
        };

        Some(color)
    }
}

#[derive(Debug, Clone)]
struct CalGray {
    white_point: [f32; 3],
    black_point: [f32; 3],
    gamma: f32,
}

// See <https://github.com/mozilla/pdf.js/blob/06f44916c8936b92f464d337fe3a0a6b2b78d5b4/src/core/colorspace.js#L752>
impl CalGray {
    fn new(dict: &Dict) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        let black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let gamma = dict.get::<f32>(GAMMA).unwrap_or(1.0);

        Some(Self {
            white_point,
            black_point,
            gamma,
        })
    }

    fn to_rgb(&self, c: f32) -> [u8; 3] {
        let g = self.gamma;
        let (_xw, yw, _zw) = {
            let wp = self.white_point;
            (wp[0], wp[1], wp[2])
        };
        let (_xb, _yb, _zb) = {
            let bp = self.black_point;
            (bp[0], bp[1], bp[2])
        };

        let a = c;
        let ag = a.powf(g);
        let l = yw * ag;
        let val = (0.0f32.max(295.8 * l.powf(0.333_333_34) - 40.8) + 0.5) as u8;

        [val, val, val]
    }
}

#[derive(Debug, Clone)]
struct CalRgb {
    white_point: [f32; 3],
    black_point: [f32; 3],
    matrix: [f32; 9],
    gamma: [f32; 3],
}

// See <https://github.com/mozilla/pdf.js/blob/06f44916c8936b92f464d337fe3a0a6b2b78d5b4/src/core/colorspace.js#L846>
// Completely copied from there without really understanding the logic, but we get the same results as Firefox
// which should be good enough (and by viewing the `calrgb.pdf` test file in different viewers you will
// see that in many cases each viewer does whatever it wants, even Acrobat), so this is good enough for us.
impl CalRgb {
    fn new(dict: &Dict) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        let black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let matrix = dict
            .get::<[f32; 9]>(MATRIX)
            .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let gamma = dict.get::<[f32; 3]>(GAMMA).unwrap_or([1.0, 1.0, 1.0]);

        Some(Self {
            white_point,
            black_point,
            matrix,
            gamma,
        })
    }

    const BRADFORD_SCALE_MATRIX: [f32; 9] = [
        0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
    ];

    const BRADFORD_SCALE_INVERSE_MATRIX: [f32; 9] = [
        0.9869929, -0.1470543, 0.1599627, 0.4323053, 0.5183603, 0.0492912, -0.0085287, 0.0400428,
        0.9684867,
    ];

    const SRGB_D65_XYZ_TO_RGB_MATRIX: [f32; 9] = [
        3.2404542, -1.5371385, -0.4985314, -0.969_266, 1.8760108, 0.0415560, 0.0556434, -0.2040259,
        1.0572252,
    ];

    const FLAT_WHITEPOINT: [f32; 3] = [1.0, 1.0, 1.0];
    const D65_WHITEPOINT: [f32; 3] = [0.95047, 1.0, 1.08883];

    fn decode_l_constant() -> f32 {
        ((8.0f32 + 16.0) / 116.0).powi(3) / 8.0
    }

    fn srgb_transfer_function(color: f32) -> f32 {
        if color <= 0.0031308 {
            (12.92 * color).clamp(0.0, 1.0)
        } else if color >= 0.99554525 {
            1.0
        } else {
            ((1.0 + 0.055) * color.powf(1.0 / 2.4) - 0.055).clamp(0.0, 1.0)
        }
    }

    fn matrix_product(a: &[f32; 9], b: &[f32; 3]) -> [f32; 3] {
        [
            a[0] * b[0] + a[1] * b[1] + a[2] * b[2],
            a[3] * b[0] + a[4] * b[1] + a[5] * b[2],
            a[6] * b[0] + a[7] * b[1] + a[8] * b[2],
        ]
    }

    fn to_flat(source_white_point: &[f32; 3], lms: &[f32; 3]) -> [f32; 3] {
        [
            lms[0] / source_white_point[0],
            lms[1] / source_white_point[1],
            lms[2] / source_white_point[2],
        ]
    }

    fn to_d65(source_white_point: &[f32; 3], lms: &[f32; 3]) -> [f32; 3] {
        [
            lms[0] * Self::D65_WHITEPOINT[0] / source_white_point[0],
            lms[1] * Self::D65_WHITEPOINT[1] / source_white_point[1],
            lms[2] * Self::D65_WHITEPOINT[2] / source_white_point[2],
        ]
    }

    fn decode_l(l: f32) -> f32 {
        if l < 0.0 {
            -Self::decode_l(-l)
        } else if l > 8.0 {
            ((l + 16.0) / 116.0).powi(3)
        } else {
            l * Self::decode_l_constant()
        }
    }

    fn compensate_black_point(source_bp: &[f32; 3], xyz_flat: &[f32; 3]) -> [f32; 3] {
        if source_bp == &[0.0, 0.0, 0.0] {
            return *xyz_flat;
        }

        let zero_decode_l = Self::decode_l(0.0);

        let mut out = [0.0; 3];
        for i in 0..3 {
            let src = Self::decode_l(source_bp[i]);
            let scale = (1.0 - zero_decode_l) / (1.0 - src);
            let offset = 1.0 - scale;
            out[i] = xyz_flat[i] * scale + offset;
        }

        out
    }

    fn normalize_white_point_to_flat(
        &self,
        source_white_point: &[f32; 3],
        xyz: &[f32; 3],
    ) -> [f32; 3] {
        if source_white_point[0] == 1.0 && source_white_point[2] == 1.0 {
            return *xyz;
        }
        let lms = Self::matrix_product(&Self::BRADFORD_SCALE_MATRIX, xyz);
        let lms_flat = Self::to_flat(source_white_point, &lms);
        Self::matrix_product(&Self::BRADFORD_SCALE_INVERSE_MATRIX, &lms_flat)
    }

    fn normalize_white_point_to_d65(
        &self,
        source_white_point: &[f32; 3],
        xyz: &[f32; 3],
    ) -> [f32; 3] {
        let lms = Self::matrix_product(&Self::BRADFORD_SCALE_MATRIX, xyz);
        let lms_d65 = Self::to_d65(source_white_point, &lms);
        Self::matrix_product(&Self::BRADFORD_SCALE_INVERSE_MATRIX, &lms_d65)
    }

    fn to_rgb(&self, mut c: [f32; 3]) -> [u8; 3] {
        for i in &mut c {
            *i = i.clamp(0.0, 1.0);
        }

        let [r, g, b] = c;
        let [gr, gg, gb] = self.gamma;
        let [agr, bgg, cgb] = [
            if r == 1.0 { 1.0 } else { r.powf(gr) },
            if g == 1.0 { 1.0 } else { g.powf(gg) },
            if b == 1.0 { 1.0 } else { b.powf(gb) },
        ];

        let m = &self.matrix;
        let x = m[0] * agr + m[3] * bgg + m[6] * cgb;
        let y = m[1] * agr + m[4] * bgg + m[7] * cgb;
        let z = m[2] * agr + m[5] * bgg + m[8] * cgb;
        let xyz = [x, y, z];

        let xyz_flat = self.normalize_white_point_to_flat(&self.white_point, &xyz);
        let xyz_black = Self::compensate_black_point(&self.black_point, &xyz_flat);
        let xyz_d65 = self.normalize_white_point_to_d65(&Self::FLAT_WHITEPOINT, &xyz_black);
        let srgb_xyz = Self::matrix_product(&Self::SRGB_D65_XYZ_TO_RGB_MATRIX, &xyz_d65);

        [
            (Self::srgb_transfer_function(srgb_xyz[0]) * 255.0 + 0.5) as u8,
            (Self::srgb_transfer_function(srgb_xyz[1]) * 255.0 + 0.5) as u8,
            (Self::srgb_transfer_function(srgb_xyz[2]) * 255.0 + 0.5) as u8,
        ]
    }
}

#[derive(Debug, Clone)]
struct Lab {
    white_point: [f32; 3],
    _black_point: [f32; 3],
    range: [f32; 4],
}

impl Lab {
    fn new(dict: &Dict) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        let black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let range = dict
            .get::<[f32; 4]>(RANGE)
            .unwrap_or([-100.0, 100.0, -100.0, 100.0]);

        Some(Self {
            white_point,
            _black_point: black_point,
            range,
        })
    }

    fn fn_g(x: f32) -> f32 {
        if x >= 6.0 / 29.0 {
            x.powi(3)
        } else {
            (108.0 / 841.0) * (x - 4.0 / 29.0)
        }
    }

    fn to_rgb(&self, c: [f32; 3]) -> [u8; 3] {
        let (l, a, b) = (c[0], c[1], c[2]);

        let m = (l + 16.0) / 116.0;
        let l = m + a / 500.0;
        let n = m - b / 200.0;

        let x = self.white_point[0] * Self::fn_g(l);
        let y = self.white_point[1] * Self::fn_g(m);
        let z = self.white_point[2] * Self::fn_g(n);

        let (r, g, b) = if self.white_point[2] < 1.0 {
            (
                x * 3.1339 + y * -1.617 + z * -0.4906,
                x * -0.9785 + y * 1.916 + z * 0.0333,
                x * 0.072 + y * -0.229 + z * 1.4057,
            )
        } else {
            (
                x * 3.2406 + y * -1.5372 + z * -0.4986,
                x * -0.9689 + y * 1.8758 + z * 0.0415,
                x * 0.0557 + y * -0.204 + z * 1.057,
            )
        };

        let conv = |v: f32| (v.max(0.0).sqrt() * 255.0).clamp(0.0, 255.0) as u8;

        [conv(r), conv(g), conv(b)]
    }
}

#[derive(Debug, Clone)]
struct Indexed {
    values: Vec<Vec<f32>>,
    hival: u8,
    base: Box<ColorSpace>,
}

impl Indexed {
    fn new(array: &Array, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip name
        let _ = iter.next::<Name>()?;
        let base_color_space = ColorSpace::new(iter.next::<Object>()?, cache)?;
        let hival = iter.next::<u8>()?;

        let values = {
            let data = iter
                .next::<Stream>()
                .and_then(|s| s.decoded().ok())
                .or_else(|| iter.next::<object::String>().map(|s| s.get().to_vec()))?;

            let num_components = base_color_space.num_components();

            let mut byte_iter = data.iter().copied();

            let mut vals = vec![];
            for _ in 0..=hival {
                let mut temp = vec![];

                for _ in 0..num_components {
                    temp.push(byte_iter.next()? as f32 / 255.0)
                }

                vals.push(temp);
            }

            vals
        };

        Some(Self {
            values,
            hival,
            base: Box::new(base_color_space),
        })
    }

    pub fn to_rgb(&self, val: f32, opacity: f32) -> AlphaColor {
        let idx = (val.clamp(0.0, self.hival as f32) + 0.5) as usize;
        self.base.to_rgba(self.values[idx].as_slice(), opacity)
    }
}

#[derive(Debug, Clone)]
struct Separation {
    alternate_space: ColorSpace,
    tint_transform: Function,
}

impl Separation {
    fn new(array: &Array, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/Separation`
        let _ = iter.next::<Name>()?;
        let name = iter.next::<Name>()?;
        let alternate_space = ColorSpace::new(iter.next::<Object>()?, cache)?;
        let tint_transform = Function::new(&iter.next::<Object>()?)?;

        if matches!(name.as_str(), "All" | "None") {
            warn!("Separation color spaces with `All` or `None` as name are not supported yet");
        }

        Some(Self {
            alternate_space,
            tint_transform,
        })
    }

    fn to_rgba(&self, c: f32, opacity: f32) -> AlphaColor {
        let res = self
            .tint_transform
            .eval(smallvec![c])
            .unwrap_or(self.alternate_space.initial_color());

        self.alternate_space.to_rgba(&res, opacity)
    }
}

#[derive(Debug, Clone)]
struct DeviceN {
    alternate_space: ColorSpace,
    num_components: usize,
    tint_transform: Function,
}

impl DeviceN {
    fn new(array: &Array, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/DeviceN`
        let _ = iter.next::<Name>()?;
        // Skip `Name`.
        let num_components = iter.next::<Array>()?.iter::<Name>().count();
        let alternate_space = ColorSpace::new(iter.next::<Object>()?, cache)?;
        let tint_transform = Function::new(&iter.next::<Object>()?)?;

        Some(Self {
            alternate_space,
            num_components,
            tint_transform,
        })
    }

    fn to_rgba(&self, c: &[f32], opacity: f32) -> AlphaColor {
        let res = self
            .tint_transform
            .eval(c.to_smallvec())
            .unwrap_or(self.alternate_space.initial_color());
        self.alternate_space.to_rgba(&res, opacity)
    }
}

struct ICCColorRepr {
    transform: Transform,
    number_components: usize,
}

#[derive(Clone)]
struct ICCProfile(Arc<ICCColorRepr>);

impl Debug for ICCProfile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ICCColor {{..}}")
    }
}

impl ICCProfile {
    fn new(profile: &[u8], number_components: usize) -> Option<Self> {
        let input = qcms::Profile::new_from_slice(profile, false)?;
        let mut output = qcms::Profile::new_sRGB();
        output.precache_output_transform();

        let data_type = match number_components {
            1 => qcms::DataType::Gray8,
            3 => qcms::DataType::RGB8,
            4 => qcms::DataType::CMYK,
            _ => {
                warn!("unsupported number of components {number_components} for ICC profile");

                return None;
            }
        };

        let transform = Transform::new_to(
            &input,
            &output,
            data_type,
            qcms::DataType::RGB8,
            qcms::Intent::default(),
        )?;

        Some(Self(Arc::new(ICCColorRepr {
            transform,
            number_components,
        })))
    }

    fn to_rgb(&self, c: &[f32]) -> Option<[u8; 3]> {
        let mut srgb = [0, 0, 0];

        match self.0.number_components {
            1 => self
                .0
                .transform
                .convert(&[f32_to_u8(*c.first()?)], &mut srgb),
            3 => self.0.transform.convert(
                &[
                    f32_to_u8(*c.first()?),
                    f32_to_u8(*c.get(1)?),
                    f32_to_u8(*c.get(2)?),
                ],
                &mut srgb,
            ),
            4 => self.0.transform.convert(
                &[
                    f32_to_u8(*c.first()?),
                    f32_to_u8(*c.get(1)?),
                    f32_to_u8(*c.get(2)?),
                    f32_to_u8(*c.get(3)?),
                ],
                &mut srgb,
            ),
            _ => return None,
        }

        Some(srgb)
    }
}

fn f32_to_u8(val: f32) -> u8 {
    (val * 255.0 + 0.5) as u8
}

#[derive(Debug, Clone)]
/// A color.
pub struct Color {
    color_space: ColorSpace,
    components: ColorComponents,
    opacity: f32,
}

impl Color {
    pub(crate) fn new(color_space: ColorSpace, components: ColorComponents, opacity: f32) -> Self {
        Self {
            color_space,
            components,
            opacity,
        }
    }

    /// Return the color as an RGBA color.
    pub fn to_rgba(&self) -> AlphaColor {
        self.color_space.to_rgba(&self.components, self.opacity)
    }
}

static CMYK_TRANSFORM: LazyLock<ICCProfile> = LazyLock::new(|| {
    ICCProfile::new(include_bytes!("../assets/CGATS001Compat-v2-micro.icc"), 4).unwrap()
});
