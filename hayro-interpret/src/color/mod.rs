//! PDF colors and color spaces.

mod cal_gray;
mod cal_rgb;
mod device_cmyk;
mod device_gray;
mod device_n;
mod device_rgb;
mod icc;
mod indexed;
mod lab;
mod pattern;
mod separation;

use self::cal_gray::CalGray;
use self::cal_rgb::CalRgb;
use self::device_cmyk::DeviceCmyk;
use self::device_gray::DeviceGray;
use self::device_n::DeviceN;
use self::device_rgb::DeviceRgb;
use self::icc::ICCProfile;
use self::indexed::Indexed;
use self::lab::Lab;
use self::pattern::Pattern;
use self::separation::Separation;
use crate::cache::{Cache, CacheKey};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use smallvec::{SmallVec, smallvec};
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

/// A storage for the components of colors.
pub type ColorComponents = SmallVec<[f32; 4]>;

#[derive(Clone)]
pub(super) struct U8Lookup<T>(Arc<OnceLock<Option<Box<[T; 256]>>>>);

impl<T> Default for U8Lookup<T> {
    fn default() -> Self {
        Self(Arc::new(OnceLock::new()))
    }
}

impl<T> std::fmt::Debug for U8Lookup<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("U8Lookup")
    }
}

impl<T> U8Lookup<T> {
    pub(super) fn get_or_init_with(
        &self,
        init: impl FnOnce(&[u8; 256]) -> Option<Box<[T; 256]>>,
    ) -> Option<&[T; 256]> {
        self.0
            .get_or_init(|| {
                let input: [u8; 256] = core::array::from_fn(|index| index as u8);
                init(&input)
            })
            .as_deref()
    }
}

impl U8Lookup<[u8; 3]> {
    pub(super) fn get_or_init(
        &self,
        convert: impl FnOnce(&[u8], &mut [u8]) -> Option<()>,
    ) -> Option<&[[u8; 3]; 256]> {
        self.get_or_init_with(|input| {
            let mut output = Box::new([[0; 3]; 256]);
            convert(input, output.as_flattened_mut())?;
            Some(output)
        })
    }
}

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
pub(crate) enum ColorSpaceType {
    DeviceCmyk(DeviceCmyk),
    DeviceGray(DeviceGray),
    DeviceRgb(DeviceRgb),
    Pattern(Pattern),
    Indexed(Indexed),
    ICCBased(ICCProfile),
    CalGray(CalGray),
    CalRgb(CalRgb),
    Lab(Lab),
    Separation(Separation),
    DeviceN(DeviceN),
}

impl ColorSpaceType {
    fn new(object: Object<'_>, cache: &Cache) -> Option<Self> {
        Self::new_inner(object, cache)
    }

    fn new_inner(object: Object<'_>, cache: &Cache) -> Option<Self> {
        if let Object::Name(name) = object {
            return Self::new_from_name(&name);
        } else if let Object::Array(color_array) = object {
            let mut iter = color_array.flex_iter();
            let name = iter.next::<Name<'_>>()?;

            match name.deref() {
                ICC_BASED => {
                    let icc_stream = iter.next::<Stream<'_>>()?;
                    let dict = icc_stream.dict();
                    let num_components = dict.get::<usize>(N)?;

                    return cache.get_or_insert_with(icc_stream.cache_key(), || {
                        if let Some(decoded) = icc_stream.decoded().ok().as_ref() {
                            ICCProfile::new(decoded, num_components)
                                .map(|icc| {
                                    // TODO: For SVG and PNG we can assume that the output color space is
                                    // sRGB. If we ever implement PDF-to-PDF, we probably want to
                                    // let the user pass the native color type and don't make this optimization
                                    // if it's not sRGB.
                                    if icc.is_srgb() {
                                        Self::DeviceRgb(DeviceRgb)
                                    } else {
                                        Self::ICCBased(icc)
                                    }
                                })
                                .or_else(|| {
                                    dict.get::<Object<'_>>(ALTERNATE)
                                        .and_then(|o| Self::new(o, cache))
                                })
                                .or_else(|| match dict.get::<u8>(N) {
                                    Some(1) => Some(Self::DeviceGray(DeviceGray)),
                                    Some(3) => Some(Self::DeviceRgb(DeviceRgb)),
                                    Some(4) => Some(Self::DeviceCmyk(DeviceCmyk)),
                                    _ => None,
                                })
                        } else {
                            None
                        }
                    });
                }
                CALCMYK => return Some(Self::DeviceCmyk(DeviceCmyk)),
                CALGRAY => {
                    let cal_dict = iter.next::<Dict<'_>>()?;
                    return Some(Self::CalGray(CalGray::new(&cal_dict)?));
                }
                CALRGB => {
                    let cal_dict = iter.next::<Dict<'_>>()?;
                    return Some(Self::CalRgb(CalRgb::new(&cal_dict)?));
                }
                DEVICE_RGB | RGB => return Some(Self::DeviceRgb(DeviceRgb)),
                DEVICE_GRAY | G => return Some(Self::DeviceGray(DeviceGray)),
                DEVICE_CMYK | CMYK => return Some(Self::DeviceCmyk(DeviceCmyk)),
                LAB => {
                    let lab_dict = iter.next::<Dict<'_>>()?;
                    return Some(Self::Lab(Lab::new(&lab_dict)?));
                }
                INDEXED | I => {
                    return Some(Self::Indexed(Indexed::new(&color_array, cache)?));
                }
                SEPARATION => {
                    return Some(Self::Separation(Separation::new(&color_array, cache)?));
                }
                DEVICE_N => {
                    return Some(Self::DeviceN(DeviceN::new(&color_array, cache)?));
                }
                PATTERN => {
                    let _ = iter.next::<Name<'_>>();
                    let cs = iter
                        .next::<Object<'_>>()
                        .and_then(|o| ColorSpace::new(o, cache))
                        .unwrap_or(ColorSpace::device_rgb());
                    return Some(Self::Pattern(Pattern::new(cs)));
                }
                _ => {
                    warn!("unsupported color space: {}", name.as_str());
                    return None;
                }
            }
        }

        None
    }

    fn new_from_name(name: &Name<'_>) -> Option<Self> {
        match name.deref() {
            DEVICE_RGB | RGB => Some(Self::DeviceRgb(DeviceRgb)),
            DEVICE_GRAY | G => Some(Self::DeviceGray(DeviceGray)),
            DEVICE_CMYK | CMYK => Some(Self::DeviceCmyk(DeviceCmyk)),
            CALCMYK => Some(Self::DeviceCmyk(DeviceCmyk)),
            PATTERN => Some(Self::Pattern(Pattern::new(ColorSpace::device_rgb()))),
            _ => None,
        }
    }
}

/// A PDF color space.
#[derive(Debug, Clone)]
pub struct ColorSpace(Arc<ColorSpaceType>);

impl ColorSpace {
    /// Create a new color space from the given object.
    pub(crate) fn new(object: Object<'_>, cache: &Cache) -> Option<Self> {
        Some(Self(Arc::new(ColorSpaceType::new(object, cache)?)))
    }

    /// Create a new color space from the name.
    pub(crate) fn new_from_name(name: &Name<'_>) -> Option<Self> {
        ColorSpaceType::new_from_name(name).map(|c| Self(Arc::new(c)))
    }

    /// Return the device gray color space.
    pub(crate) fn device_gray() -> Self {
        Self(Arc::new(ColorSpaceType::DeviceGray(DeviceGray)))
    }

    /// Return the device RGB color space.
    pub(crate) fn device_rgb() -> Self {
        Self(Arc::new(ColorSpaceType::DeviceRgb(DeviceRgb)))
    }

    /// Return the device CMYK color space.
    pub(crate) fn device_cmyk() -> Self {
        Self(Arc::new(ColorSpaceType::DeviceCmyk(DeviceCmyk)))
    }

    /// Return the pattern color space.
    pub(crate) fn pattern() -> Self {
        Self(Arc::new(ColorSpaceType::Pattern(Pattern::new(
            Self::device_gray(),
        ))))
    }

    pub(crate) fn pattern_cs(&self) -> Option<Self> {
        match self.0.as_ref() {
            ColorSpaceType::Pattern(pattern) => Some(pattern.color_space()),
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

    pub(crate) fn indexed_hival(&self) -> Option<u8> {
        match self.0.as_ref() {
            ColorSpaceType::Indexed(indexed) => Some(indexed.hival()),
            ColorSpaceType::Pattern(pattern) => pattern.color_space().indexed_hival(),
            _ => None,
        }
    }

    /// Get the default decode array for the color space.
    pub(crate) fn default_decode_arr(&self, n: f32) -> SmallVec<[(f32, f32); 4]> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(_) => {
                smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0), (0.0, 1.0)]
            }
            ColorSpaceType::DeviceGray(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::DeviceRgb(_) => smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0)],
            ColorSpaceType::ICCBased(i) => smallvec![(0.0, 1.0); i.number_components()],
            ColorSpaceType::CalGray(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::CalRgb(_) => smallvec![(0.0, 1.0), (0.0, 1.0), (0.0, 1.0)],
            ColorSpaceType::Lab(l) => smallvec![
                (0.0, 100.0),
                (l.range[0], l.range[1]),
                (l.range[2], l.range[3]),
            ],
            ColorSpaceType::Indexed(_) => smallvec![(0.0, 2.0_f32.powf(n) - 1.0)],
            ColorSpaceType::Separation(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::DeviceN(d) => smallvec![(0.0, 1.0); d.num_components as usize],
            // Not a valid image color space.
            ColorSpaceType::Pattern(_) => smallvec![(0.0, 1.0)],
        }
    }

    pub(crate) fn inverted_default_decode_arr(&self, n: f32) -> SmallVec<[(f32, f32); 4]> {
        self.default_decode_arr(n)
            .iter()
            .map(|(min, max)| (*max, *min))
            .collect()
    }

    pub(crate) fn component_ranges(&self) -> SmallVec<[(f32, f32); 4]> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(_) => smallvec![(0.0, 1.0); 4],
            ColorSpaceType::DeviceGray(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::DeviceRgb(_) => smallvec![(0.0, 1.0); 3],
            ColorSpaceType::ICCBased(i) => smallvec![(0.0, 1.0); i.number_components()],
            ColorSpaceType::CalGray(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::CalRgb(_) => smallvec![(0.0, 1.0); 3],
            ColorSpaceType::Lab(_) => {
                smallvec![(0.0, 100.0), (-128.0, 127.0), (-128.0, 127.0)]
            }
            ColorSpaceType::Indexed(i) => smallvec![(0.0, i.hival() as f32)],
            ColorSpaceType::Separation(_) => smallvec![(0.0, 1.0)],
            ColorSpaceType::Pattern(pattern) => pattern.color_space().component_ranges(),
            ColorSpaceType::DeviceN(d) => smallvec![(0.0, 1.0); d.num_components as usize],
        }
    }

    pub(crate) fn convert_values(&self, input: &[f32], output: &mut [u8]) -> Option<()> {
        let converted = self.encode_values(input);
        self.convert(&converted, output)
    }

    pub(crate) fn encode_values(&self, input: &[f32]) -> SmallVec<[u8; 4]> {
        if let Some(hival) = self.indexed_hival() {
            return input
                .iter()
                .map(|value| (*value + 0.5).clamp(0.0, hival as f32) as u8)
                .collect();
        }

        let ranges = match self.0.as_ref() {
            ColorSpaceType::ICCBased(icc) if icc.is_lab() => {
                smallvec![(0.0, 100.0), (-128.0, 127.0), (-128.0, 127.0)]
            }
            _ => self.component_ranges(),
        };
        encode_components(input, &ranges)
    }

    /// Get the initial color of the color space.
    pub(crate) fn initial_color(&self) -> ColorComponents {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(_) => smallvec![0.0, 0.0, 0.0, 1.0],
            ColorSpaceType::DeviceGray(_) => smallvec![0.0],
            ColorSpaceType::DeviceRgb(_) => smallvec![0.0, 0.0, 0.0],
            ColorSpaceType::ICCBased(icc) => match icc.number_components() {
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
            ColorSpaceType::Pattern(pattern) => pattern.initial_color(),
            ColorSpaceType::DeviceN(d) => smallvec![1.0; d.num_components as usize],
        }
    }

    /// Get the number of components of the color space.
    pub(crate) fn num_components(&self) -> u8 {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(_) => 4,
            ColorSpaceType::DeviceGray(_) => 1,
            ColorSpaceType::DeviceRgb(_) => 3,
            ColorSpaceType::ICCBased(icc) => icc.number_components() as u8,
            ColorSpaceType::CalGray(_) => 1,
            ColorSpaceType::CalRgb(_) => 3,
            ColorSpaceType::Lab(_) => 3,
            ColorSpaceType::Indexed(_) => 1,
            ColorSpaceType::Separation(_) => 1,
            ColorSpaceType::Pattern(pattern) => pattern.num_components(),
            ColorSpaceType::DeviceN(d) => d.num_components,
        }
    }

    /// Turn the given component values and opacity into an RGBA color.
    #[inline]
    pub fn to_rgba(&self, c: &[f32], opacity: f32) -> AlphaColor {
        let alpha = f32_to_u8(opacity);

        match self.0.as_ref() {
            ColorSpaceType::DeviceGray(_) => {
                let gray = c.first().copied().map(f32_to_u8).unwrap_or(0);
                AlphaColor::from_rgba8(gray, gray, gray, alpha)
            }
            ColorSpaceType::DeviceRgb(_) => AlphaColor::from_rgba8(
                c.first().copied().map(f32_to_u8).unwrap_or(0),
                c.get(1).copied().map(f32_to_u8).unwrap_or(0),
                c.get(2).copied().map(f32_to_u8).unwrap_or(0),
                alpha,
            ),
            ColorSpaceType::DeviceCmyk(device_cmyk) if c.len() == 4 => {
                let input = [
                    f32_to_u8(c[0]),
                    f32_to_u8(c[1]),
                    f32_to_u8(c[2]),
                    f32_to_u8(c[3]),
                ];
                let mut output = [0; 3];

                if device_cmyk.convert(&input, &mut output).is_some() {
                    AlphaColor::from_rgba8(output[0], output[1], output[2], alpha)
                } else {
                    AlphaColor::BLACK
                }
            }
            _ => self.to_alpha_color(c, opacity).unwrap_or(AlphaColor::BLACK),
        }
    }

    fn to_alpha_color(&self, input: &[f32], mut opacity: f32) -> Option<AlphaColor> {
        let mut output = [0; 3];
        self.convert_values(input, &mut output)?;

        // For separation color spaces:
        // "The special colourant name None shall not produce any visible output.
        // Painting operations in a Separation space with this colourant name
        // shall have no effect on the current page."
        if self.is_none() {
            opacity = 0.0;
        }

        Some(AlphaColor::from_rgba8(
            output[0],
            output[1],
            output[2],
            (opacity * 255.0 + 0.5) as u8,
        ))
    }
}

impl ToRgb for ColorSpace {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(i) => i.convert(input, output),
            ColorSpaceType::DeviceGray(i) => i.convert(input, output),
            ColorSpaceType::DeviceRgb(i) => i.convert(input, output),
            ColorSpaceType::Pattern(i) => i.convert(input, output),
            ColorSpaceType::Indexed(i) => i.convert(input, output),
            ColorSpaceType::ICCBased(i) => i.convert(input, output),
            ColorSpaceType::CalGray(i) => i.convert(input, output),
            ColorSpaceType::CalRgb(i) => i.convert(input, output),
            ColorSpaceType::Lab(i) => i.convert(input, output),
            ColorSpaceType::Separation(i) => i.convert(input, output),
            ColorSpaceType::DeviceN(i) => i.convert(input, output),
        }
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceCmyk(i) => i.convert_in_place(input),
            ColorSpaceType::DeviceGray(i) => i.convert_in_place(input),
            ColorSpaceType::DeviceRgb(i) => i.convert_in_place(input),
            ColorSpaceType::Pattern(i) => i.convert_in_place(input),
            ColorSpaceType::Indexed(i) => i.convert_in_place(input),
            ColorSpaceType::ICCBased(i) => i.convert_in_place(input),
            ColorSpaceType::CalGray(i) => i.convert_in_place(input),
            ColorSpaceType::CalRgb(i) => i.convert_in_place(input),
            ColorSpaceType::Lab(i) => i.convert_in_place(input),
            ColorSpaceType::Separation(i) => i.convert_in_place(input),
            ColorSpaceType::DeviceN(i) => i.convert_in_place(input),
        }
    }

    fn is_none(&self) -> bool {
        match self.0.as_ref() {
            ColorSpaceType::Separation(s) => s.is_none(),
            ColorSpaceType::DeviceN(d) => d.is_none(),
            _ => false,
        }
    }
}

impl ToLuma for ColorSpace {
    fn to_luma(&self, input: &mut [u8]) -> Option<()> {
        match self.0.as_ref() {
            ColorSpaceType::DeviceGray(i) => i.to_luma(input),
            ColorSpaceType::Pattern(i) => i.to_luma(input),
            ColorSpaceType::Indexed(i) => i.to_luma(input),
            ColorSpaceType::ICCBased(i) => i.to_luma(input),
            ColorSpaceType::CalGray(i) => i.to_luma(input),
            ColorSpaceType::DeviceCmyk(_)
            | ColorSpaceType::DeviceRgb(_)
            | ColorSpaceType::CalRgb(_)
            | ColorSpaceType::Lab(_)
            | ColorSpaceType::Separation(_)
            | ColorSpaceType::DeviceN(_) => None,
        }
    }
}

#[inline(always)]
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
    #[inline]
    pub fn to_rgba(&self) -> AlphaColor {
        self.color_space.to_rgba(&self.components, self.opacity)
    }

    /// Create a color from RGBA.
    #[inline]
    pub fn from_rgba(rgba: AlphaColor) -> Self {
        let c = rgba.components();
        Self {
            color_space: ColorSpace::device_rgb(),
            components: smallvec![c[0], c[1], c[2]],
            opacity: c[3],
        }
    }
}

pub(crate) trait ToRgb {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()>;
    fn convert_in_place(&self, _input: &mut [u8]) -> Option<()> {
        None
    }
    fn is_none(&self) -> bool {
        false
    }
}

pub(crate) trait ToLuma {
    fn to_luma(&self, input: &mut [u8]) -> Option<()>;
}

#[inline]
fn encode_components(input: &[f32], ranges: &[(f32, f32)]) -> SmallVec<[u8; 4]> {
    input
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let (min, max) = ranges[index % ranges.len()];
            (((*value - min) / (max - min)) * 255.0 + 0.5) as u8
        })
        .collect()
}
