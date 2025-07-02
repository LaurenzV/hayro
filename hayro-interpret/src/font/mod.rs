use crate::Paint;
use crate::cache::Cache;
use crate::context::Context;
use crate::device::Device;
use crate::font::cid::Type0Font;
use crate::font::generated::{mac_expert, mac_os_roman, mac_roman, standard, win_ansi};
use crate::font::true_type::TrueTypeFont;
use crate::font::type1::Type1Font;
use crate::font::type3::Type3;
use crate::interpret::state::State;
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::SUBTYPE;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::name::Name;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Vec2};
use outline::OutlineFont;
use skrifa::GlyphId;
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

mod blob;
mod cid;
mod generated;
mod glyph_simulator;
pub(crate) mod outline;
mod standard_font;
mod true_type;
mod type1;
pub(crate) mod type3;

pub(crate) const UNITS_PER_EM: f32 = 1000.0;

/// A glyph that can be drawn.
pub enum Glyph<'a> {
    /// A glyph defined by an outline.
    Outline(OutlineGlyph),
    /// A glyph defined by PDF drawing instructions.
    Type3(Type3Glyph<'a>),
}

impl Glyph<'_> {
    pub(crate) fn glyph_transform(&self) -> Affine {
        match self {
            Glyph::Outline(o) => o.glyph_transform,
            Glyph::Type3(s) => s.glyph_transform,
        }
    }
}

/// A glyph defined by an outline.
#[derive(Clone, Debug)]
pub struct OutlineGlyph {
    pub(crate) id: GlyphId,
    pub(crate) font: OutlineFont,
    /// A transform that should be applied to the glyph before drawing.
    pub glyph_transform: Affine,
}

impl OutlineGlyph {
    /// Return the outline of the glyph, assuming an upem value of 1000.
    pub fn outline(&self) -> BezPath {
        self.font.outline_glyph(self.id)
    }
}

pub struct Type3Glyph<'a> {
    pub(crate) font: Arc<Type3<'a>>,
    pub(crate) glyph_id: GlyphId,
    pub(crate) state: State<'a>,
    pub(crate) parent_resources: Resources<'a>,
    pub(crate) cache: Cache,
    pub(crate) glyph_transform: Affine,
    pub(crate) xref: &'a XRef,
}

/// A glyph defined by PDF drawing instructions.
impl<'a> Type3Glyph<'a> {
    /// Draw the type3 glyph to the given device.
    pub fn interpret(&self, device: &mut impl Device, paint: &Paint) {
        self.font.render_glyph(self, paint, device);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Font<'a>(FontType<'a>);

impl<'a> Font<'a> {
    pub(crate) fn new(dict: &Dict<'a>) -> Option<Self> {
        let f_type = match dict.get::<Name>(SUBTYPE)?.deref() {
            TYPE1 | MM_TYPE1 => FontType::Type1(Arc::new(Type1Font::new(dict)?)),
            TRUE_TYPE => TrueTypeFont::new(dict)
                .map(Arc::new)
                .map(FontType::TrueType)
                .or_else(|| Type1Font::new(dict).map(Arc::new).map(FontType::Type1))?,
            TYPE0 => FontType::Type0(Arc::new(Type0Font::new(dict)?)),
            TYPE3 => FontType::Type3(Arc::new(Type3::new(dict))),
            f => {
                println!(
                    "unimplemented font type {:?}",
                    std::str::from_utf8(f).unwrap_or("unknown type")
                );

                return None;
            }
        };

        Some(Self(f_type))
    }

    pub(crate) fn map_code(&self, code: u16) -> GlyphId {
        match &self.0 {
            FontType::Type1(f) => {
                debug_assert!(code <= u8::MAX as u16);

                f.map_code(code as u8)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u16);

                t.map_code(code as u8)
            }
            FontType::Type0(t) => t.map_code(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u16);

                t.map_code(code as u8)
            }
        }
    }

    pub(crate) fn get_glyph(
        &self,
        glyph: GlyphId,
        ctx: &mut Context<'a>,
        resources: &Resources<'a>,
        origin_displacement: Vec2,
    ) -> Glyph<'a> {
        let glyph_transform = ctx.get().text_state.full_transform()
            * Affine::scale(1.0 / UNITS_PER_EM as f64)
            * Affine::translate(origin_displacement);

        match &self.0 {
            FontType::Type1(t) => {
                let font = OutlineFont::Type1(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::TrueType(t) => {
                let font = OutlineFont::TrueType(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::Type0(t) => {
                let font = OutlineFont::Type0(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::Type3(t) => {
                let shape_glyph = Type3Glyph {
                    font: t.clone(),
                    glyph_id: glyph,
                    state: ctx.get().clone(),
                    parent_resources: resources.clone(),
                    cache: ctx.object_cache.clone(),
                    xref: ctx.xref,
                    glyph_transform,
                };

                Glyph::Type3(shape_glyph)
            }
        }
    }

    pub(crate) fn code_advance(&self, code: u16) -> Vec2 {
        match &self.0 {
            FontType::Type1(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8).unwrap_or(0.0) as f64, 0.0)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
            FontType::Type0(t) => t.code_advance(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
        }
    }

    pub(crate) fn origin_displacement(&self, code: u16) -> Vec2 {
        match &self.0 {
            FontType::Type1(_) => Vec2::default(),
            FontType::TrueType(_) => Vec2::default(),
            FontType::Type0(t) => t.origin_displacement(code),
            FontType::Type3(_) => Vec2::default(),
        }
    }

    pub(crate) fn code_len(&self) -> usize {
        match &self.0 {
            FontType::Type1(_) => 1,
            FontType::TrueType(_) => 1,
            FontType::Type0(t) => t.code_len(),
            FontType::Type3(_) => 1,
        }
    }

    pub(crate) fn is_horizontal(&self) -> bool {
        match &self.0 {
            FontType::Type1(_) => true,
            FontType::TrueType(_) => true,
            FontType::Type0(t) => t.is_horizontal(),
            FontType::Type3(_) => true,
        }
    }
}

#[derive(Clone, Debug)]
enum FontType<'a> {
    Type1(Arc<Type1Font>),
    TrueType(Arc<TrueTypeFont>),
    Type0(Arc<Type0Font>),
    Type3(Arc<Type3<'a>>),
}

#[derive(Debug)]
enum Encoding {
    Standard,
    MacRoman,
    WinAnsi,
    MacExpert,
    BuiltIn,
}

impl Encoding {
    fn map_code(&self, code: u8) -> Option<&'static str> {
        if code == 0 {
            return Some(".notdef");
        }
        match self {
            Encoding::Standard => standard::get(code),
            Encoding::MacRoman => mac_roman::get(code).or_else(|| mac_os_roman::get(code)),
            Encoding::WinAnsi => win_ansi::get(code),
            Encoding::MacExpert => mac_expert::get(code),
            Encoding::BuiltIn => None,
        }
    }
}
