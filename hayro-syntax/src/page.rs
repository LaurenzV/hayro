//! Reading the pages of a PDF document.

use crate::content::{TypedIter, UntypedIter};
use crate::object::Array;
use crate::object::Dict;
use crate::object::Name;
use crate::object::Rect;
use crate::object::Stream;
use crate::object::array::ArrayIter;
use crate::object::dict::keys::*;
use crate::object::{MaybeRef, Object, ObjectIdentifier};
use crate::reader::ReaderContext;
use crate::sync::{FxHashMap, FxHashSet, OnceLock};
use crate::transform::Transform;
use crate::util::FloatExt;
use crate::xref::XRef;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Deref;

#[derive(Debug, Clone)]
struct PagesContext<'a> {
    media_box: Option<Rect>,
    crop_box: Option<Rect>,
    rotate: Option<i32>,
    resources: Resources<'a>,
}

impl<'a> PagesContext<'a> {
    fn new() -> Self {
        Self {
            media_box: None,
            crop_box: None,
            rotate: None,
            resources: Resources::new(Dict::empty()),
        }
    }

    fn apply(&mut self, dict: &Dict<'a>) {
        self.media_box = dict.get::<Rect>(MEDIA_BOX).or(self.media_box);
        self.crop_box = dict.get::<Rect>(CROP_BOX).or(self.crop_box);
        self.rotate = dict.get::<i32>(ROTATE).or(self.rotate);
        if let Some(resources) = dict.get::<Dict<'a>>(RESOURCES) {
            self.resources = Resources::new(resources);
        }
    }

    fn from_parents(
        leaf: &Dict<'a>,
        xref: &'a XRef,
        cache: &mut FxHashMap<ObjectIdentifier, Self>,
    ) -> Self {
        let mut ctx = Self::new();
        let mut ancestors: Vec<(ObjectIdentifier, Dict<'a>)> = Vec::new();
        let mut visited = FxHashSet::default();
        let leaf_id = leaf.obj_id();

        let mut parent = leaf.get_ref(PARENT);
        while let Some(id) = parent.map(ObjectIdentifier::from) {
            if let Some(inherited) = cache.get(&id) {
                ctx = inherited.clone();
                break;
            }
            let at_leaf = Some(id) == leaf_id;
            if at_leaf || !visited.insert(id) {
                if at_leaf && leaf.get::<Name<'_>>(TYPE).as_deref() == Some(PAGE) {
                    break;
                }
                // Seed the reverse fold with a full lap so every cached cycle entry sees every attribute.
                for (ancestor, dict) in ancestors.iter().rev() {
                    ctx.apply(dict);
                    if *ancestor == id {
                        break;
                    }
                }
                if at_leaf {
                    ctx.apply(leaf);
                }
                break;
            }

            let Some(dict) = xref.get::<Object<'_>>(id).and_then(Object::into_dict) else {
                cache.insert(id, Self::new());
                break;
            };
            if dict.get::<Name<'_>>(TYPE).as_deref() == Some(PAGE) {
                cache.insert(id, Self::new());
                break;
            }
            parent = dict.get_ref(PARENT);
            ancestors.push((id, dict));
        }

        for (id, dict) in ancestors.into_iter().rev() {
            ctx.apply(&dict);
            cache.insert(id, ctx.clone());
        }
        ctx
    }
}

/// A structure holding the pages of a PDF document.
pub struct Pages<'a> {
    pages: Vec<Page<'a>>,
    xref: &'a XRef,
}

impl<'a> Pages<'a> {
    pub(crate) fn new(
        pages_dict: &Dict<'a>,
        ctx: &ReaderContext<'a>,
        xref: &'a XRef,
    ) -> Option<Self> {
        let mut pages = vec![];
        let pages_ctx = PagesContext::new();
        resolve_pages(pages_dict, &mut pages, pages_ctx, ctx)?;

        Some(Self { pages, xref })
    }

    pub(crate) fn new_brute_force(ctx: &ReaderContext<'a>, xref: &'a XRef) -> Option<Self> {
        let mut pages = vec![];
        let mut cache = FxHashMap::default();

        for object in xref.objects() {
            if let Some(dict) = object.into_dict()
                && dict.contains_key(CONTENTS)
            {
                let inherited = PagesContext::from_parents(&dict, xref, &mut cache);
                pages.push(Page::new(&dict, inherited, ctx));
            }
        }

        if pages.is_empty() {
            return None;
        }

        Some(Self { pages, xref })
    }

    /// Return the xref table (of the document the pages belong to).
    pub fn xref(&self) -> &'a XRef {
        self.xref
    }
}

impl<'a> Deref for Pages<'a> {
    type Target = [Page<'a>];

    fn deref(&self) -> &Self::Target {
        &self.pages
    }
}

struct PageTreeFrame<'a> {
    kids: ArrayIter<'a>,
    ctx: PagesContext<'a>,
}

impl<'a> PageTreeFrame<'a> {
    fn new(dict: &Dict<'a>, mut ctx: PagesContext<'a>) -> Option<Self> {
        ctx.apply(dict);

        Some(Self {
            kids: dict.get::<Array<'a>>(KIDS)?.raw_iter(),
            ctx,
        })
    }
}

fn resolve_pages<'a>(
    pages_dict: &Dict<'a>,
    entries: &mut Vec<Page<'a>>,
    ctx: PagesContext<'a>,
    reader_ctx: &ReaderContext<'a>,
) -> Option<()> {
    let mut visited = FxHashSet::default();

    if let Some(id) = pages_dict.obj_id() {
        visited.insert(id);
    }

    let mut stack = vec![PageTreeFrame::new(pages_dict, ctx)?];

    while let Some(frame) = stack.last_mut() {
        let Some(kid) = frame.kids.next() else {
            stack.pop();

            continue;
        };

        let dict = match kid {
            MaybeRef::Ref(id) => {
                if !visited.insert(id.into()) {
                    continue;
                }

                reader_ctx
                    .xref()
                    .get_with::<Dict<'a>>(id.into(), reader_ctx)
            }
            MaybeRef::NotRef(object) => object.into_dict(),
        };

        let Some(dict) = dict else {
            continue;
        };

        match dict.get::<Name<'_>>(TYPE).as_deref() {
            Some(PAGES) => {
                if let Some(child) = PageTreeFrame::new(&dict, frame.ctx.clone()) {
                    stack.push(child);
                }
            }
            // Lenient: treat a missing or unknown `Type` as a page (corpus 0083781).
            _ => {
                entries.push(Page::new(&dict, frame.ctx.clone(), reader_ctx));
            }
        }
    }

    Some(())
}

/// The rotation of the page.
#[derive(Debug, Copy, Clone)]
pub enum Rotation {
    /// No rotation.
    None,
    /// A rotation of 90 degrees.
    Horizontal,
    /// A rotation of 180 degrees.
    Flipped,
    /// A rotation of 270 degrees.
    FlippedHorizontal,
}

/// A PDF page.
pub struct Page<'a> {
    inner: Dict<'a>,
    media_box: Rect,
    crop_box: Rect,
    rotation: Rotation,
    page_streams: OnceLock<Option<Box<[u8]>>>,
    resources: Resources<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> Page<'a> {
    fn new(dict: &Dict<'a>, mut ctx: PagesContext<'a>, reader_ctx: &ReaderContext<'a>) -> Self {
        ctx.apply(dict);
        let media_box = ctx.media_box.unwrap_or(A4);
        let crop_box = ctx.crop_box.unwrap_or(media_box);
        let rotation = match ctx.rotate.unwrap_or(0).rem_euclid(360) {
            0 => Rotation::None,
            90 => Rotation::Horizontal,
            180 => Rotation::Flipped,
            270 => Rotation::FlippedHorizontal,
            _ => Rotation::None,
        };

        Self {
            inner: dict.clone(),
            media_box,
            crop_box,
            rotation,
            page_streams: OnceLock::new(),
            resources: ctx.resources,
            ctx: reader_ctx.clone(),
        }
    }

    fn operations_impl(&self) -> Option<UntypedIter<'_>> {
        let stream = self.page_stream()?;
        let iter = UntypedIter::new(stream);

        Some(iter)
    }

    /// Return the decoded content stream of the page.
    pub fn page_stream(&self) -> Option<&[u8]> {
        let convert_single = |s: Stream<'_>| {
            let data = s.decoded_data().ok()?;
            Some(data.into_owned())
        };

        self.page_streams
            .get_or_init(|| {
                if let Some(stream) = self.inner.get::<Stream<'_>>(CONTENTS) {
                    convert_single(stream)
                } else if let Some(array) = self.inner.get::<Array<'_>>(CONTENTS) {
                    let streams = array
                        .raw_iter()
                        .filter_map(|item| item.resolve(&self.ctx)?.into_stream())
                        .flat_map(convert_single);

                    let mut collected = vec![];

                    for stream in streams {
                        collected.extend(stream);
                        // Streams must have at least one whitespace in-between.
                        collected.push(b' ');
                    }

                    Some(collected)
                } else {
                    None
                }
                .map(Vec::into_boxed_slice)
            })
            .as_deref()
    }

    /// Get the resources of the page.
    pub fn resources(&self) -> &Resources<'a> {
        &self.resources
    }

    /// Get the media box of the page.
    pub fn media_box(&self) -> Rect {
        self.media_box
    }

    /// Get the rotation of the page.
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// Get the crop box of the page.
    pub fn crop_box(&self) -> Rect {
        self.crop_box
    }

    /// Return the intersection of crop box and media box.
    pub fn intersected_crop_box(&self) -> Rect {
        self.crop_box().intersect(self.media_box())
    }

    /// Return the dimensions of the intersected crop box, or A4 for zero-area pages.
    pub fn base_dimensions(&self) -> (f32, f32) {
        let crop_box = self.intersected_crop_box();

        if (crop_box.width() as f32).is_nearly_zero() || (crop_box.height() as f32).is_nearly_zero()
        {
            (A4.width() as f32, A4.height() as f32)
        } else {
            (
                crop_box.width().max(1.0) as f32,
                crop_box.height().max(1.0) as f32,
            )
        }
    }

    /// Return the rotated width and height of the page.
    pub fn render_dimensions(&self) -> (f32, f32) {
        let (mut base_width, mut base_height) = self.base_dimensions();

        if matches!(
            self.rotation(),
            Rotation::Horizontal | Rotation::FlippedHorizontal
        ) {
            core::mem::swap(&mut base_width, &mut base_height);
        }

        (base_width, base_height)
    }

    /// Return an untyped iterator over the operators of the page's content stream.
    pub fn operations(&self) -> UntypedIter<'_> {
        self.operations_impl().unwrap_or(UntypedIter::empty())
    }

    /// Get the raw dictionary of the page.
    pub fn raw(&self) -> &Dict<'a> {
        &self.inner
    }

    /// Get the xref table (of the document the page belongs to).
    pub fn xref(&self) -> &'a XRef {
        self.ctx.xref()
    }

    /// Return a typed iterator over the operators of the page's content stream.
    pub fn typed_operations(&self) -> TypedIter<'_> {
        TypedIter::from_untyped(self.operations())
    }

    /// Return the initial transform for rendering the page.
    pub fn initial_transform(&self, invert_y: bool) -> Transform {
        let crop_box = self.intersected_crop_box();
        let (_, base_height) = self.base_dimensions();
        let (width, height) = self.render_dimensions();

        let horizontal_t = Transform::ROTATE_CW_90 * Transform::translate((0.0, -width as f64));
        let flipped_horizontal_t =
            Transform::translate((0.0, height as f64)) * Transform::ROTATE_CCW_90;

        let rotation_transform = match self.rotation() {
            Rotation::None => Transform::IDENTITY,
            Rotation::Horizontal => {
                if invert_y {
                    horizontal_t
                } else {
                    flipped_horizontal_t
                }
            }
            Rotation::Flipped => {
                Transform::scale(-1.0) * Transform::translate((-width as f64, -height as f64))
            }
            Rotation::FlippedHorizontal => {
                if invert_y {
                    flipped_horizontal_t
                } else {
                    horizontal_t
                }
            }
        };

        let inversion_transform = if invert_y {
            Transform::new([1.0, 0.0, 0.0, -1.0, 0.0, base_height as f64])
        } else {
            Transform::IDENTITY
        };

        rotation_transform
            * inversion_transform
            * Transform::translate((-crop_box.x0, -crop_box.y0))
    }
}

/// A structure keeping track of the resources of a page.
#[derive(Clone, Debug)]
pub struct Resources<'a> {
    /// The raw dictionary of external graphics states.
    pub ext_g_states: Dict<'a>,
    /// The raw dictionary of fonts.
    pub fonts: Dict<'a>,
    /// The raw dictionary of properties.
    pub properties: Dict<'a>,
    /// The raw dictionary of color spaces.
    pub color_spaces: Dict<'a>,
    /// The raw dictionary of x objects.
    pub x_objects: Dict<'a>,
    /// The raw dictionary of patterns.
    pub patterns: Dict<'a>,
    /// The raw dictionary of shadings.
    pub shadings: Dict<'a>,
}

impl<'a> Resources<'a> {
    /// Create a new `Resources` object.
    pub fn new(resources: Dict<'a>) -> Self {
        let ext_g_states = resources.get::<Dict<'_>>(EXT_G_STATE).unwrap_or_default();
        let fonts = resources.get::<Dict<'_>>(FONT).unwrap_or_default();
        let color_spaces = resources.get::<Dict<'_>>(COLORSPACE).unwrap_or_default();
        let x_objects = resources.get::<Dict<'_>>(XOBJECT).unwrap_or_default();
        let patterns = resources.get::<Dict<'_>>(PATTERN).unwrap_or_default();
        let shadings = resources.get::<Dict<'_>>(SHADING).unwrap_or_default();
        let properties = resources.get::<Dict<'_>>(PROPERTIES).unwrap_or_default();

        Self {
            ext_g_states,
            fonts,
            color_spaces,
            properties,
            x_objects,
            patterns,
            shadings,
        }
    }

    /// Get an external graphics state by name.
    pub fn get_ext_g_state(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.ext_g_states.get(name)
    }

    /// Get a color space by name.
    pub fn get_color_space(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.color_spaces.get(name)
    }

    /// Get a font by name.
    pub fn get_font(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.fonts.get(name)
    }

    /// Get a pattern by name.
    pub fn get_pattern(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.patterns.get(name)
    }

    /// Get an x object by name.
    pub fn get_x_object(&self, name: &Name<'_>) -> Option<Stream<'a>> {
        self.x_objects.get(name)
    }

    /// Get a shading by name.
    pub fn get_shading(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.shadings.get(name)
    }
}

const POINTS_PER_INCH: f64 = 72.0;
const POINTS_PER_MM: f64 = 1.0 / (10.0 * 2.54) * POINTS_PER_INCH;

/// The dimension of an A4 page.
pub const A4: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 210.0 * POINTS_PER_MM,
    y1: 297.0 * POINTS_PER_MM,
};

pub(crate) mod cached {
    use crate::page::Pages;
    use crate::reader::ReaderContext;
    use crate::xref::XRef;
    use core::ops::Deref;

    // Keep in sync with the `sync` module.
    #[cfg(feature = "std")]
    pub(crate) use std::sync::Arc;

    #[cfg(not(feature = "std"))]
    pub(crate) use alloc::rc::Rc as Arc;

    pub(crate) struct CachedPages {
        pages: Pages<'static>,
        // Must come after `pages`, which borrows from it.
        _xref: Arc<XRef>,
    }

    impl CachedPages {
        pub(crate) fn new(xref: Arc<XRef>) -> Option<Self> {
            // SAFETY:
            // - The XRef's location is stable in memory:
            //   - We wrapped it in a `Arc` (or `Rc` in `no_std`), which implements `StableDeref`.
            //   - The struct owns the `Arc`, ensuring that the inner value is not dropped during the whole
            //     duration.
            // - The internal 'static lifetime is not leaked because its rewritten
            //   to the self-lifetime in `pages()`.
            let xref_reference: &'static XRef = unsafe { core::mem::transmute(xref.deref()) };

            let ctx = ReaderContext::new(xref_reference, false);
            let pages = xref_reference
                .get_with(xref.trailer_data().pages_ref, &ctx)
                .and_then(|p| Pages::new(&p, &ctx, xref_reference))
                .or_else(|| Pages::new_brute_force(&ctx, xref_reference))?;

            Some(Self { pages, _xref: xref })
        }

        pub(crate) fn get(&self) -> &Pages<'_> {
            &self.pages
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pdf;
    use alloc::{format, string::String};
    use core::fmt::Write;

    fn damaged_pdf(objects: impl IntoIterator<Item = impl AsRef<str>>) -> Pdf {
        let mut data =
            String::from("%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        for (i, dict) in objects.into_iter().enumerate() {
            writeln!(data, "{} 0 obj\n{}\nendobj", i + 2, dict.as_ref()).unwrap();
        }
        data.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF");
        Pdf::new(data.into_bytes()).unwrap()
    }

    #[test]
    fn recovered_pages_inherit_nearest_attributes() {
        let pdf = damaged_pdf([
            "<< /Type /Pages /MediaBox [0 0 32 18] /CropBox [0 0 30 16] /Rotate 90 /Resources << /Font << /A << >> >> >> >>",
            "<< /Parent 2 0 R /Rotate 180 /Resources << >> >>",
            "<< /Contents [] /Parent 2 0 R >>",
            "<< /Contents [] /Parent 3 0 R >>",
        ]);
        let pages = pdf.pages();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].render_dimensions(), (16.0, 30.0));
        assert!(pages[0].resources().fonts.contains_key(b"A"));
        assert_eq!(pages[1].media_box().width(), 32.0);
        assert_eq!(pages[1].render_dimensions(), (30.0, 16.0));
        assert!(matches!(pages[1].rotation(), Rotation::Flipped));
        assert!(pages[1].resources().fonts.is_empty());
    }

    #[test]
    fn recovered_pages_enter_a_cycle_at_different_parents() {
        let pdf = damaged_pdf([
            "<< /Contents [] /Parent 3 0 R /Rotate 90 /MediaBox [0 0 32 18] >>",
            "<< /Parent 2 0 R /MediaBox [0 0 16 10] >>",
            "<< /Contents [] /Parent 3 0 R >>",
            "<< /Contents [] /Parent 2 0 R >>",
            "<< /Type /Page /Contents [] /Parent 6 0 R >>",
        ]);
        let sizes: Vec<_> = pdf.pages().iter().map(Page::render_dimensions).collect();
        assert_eq!(
            sizes,
            [
                (18., 32.),
                (10., 16.),
                (18., 32.),
                (A4.width() as f32, A4.height() as f32)
            ]
        );
    }

    #[test]
    fn invalid_parent_ends_inheritance() {
        for target in ["<< /Type /Page /Rotate 90 >>", "null"] {
            let pdf = damaged_pdf([
                "<< /Parent 3 0 R /MediaBox [0 0 32 18] >>",
                target,
                "<< /Contents [] /Parent 2 0 R >>",
            ]);
            assert_eq!(pdf.pages()[0].render_dimensions(), (32., 18.));
        }
    }

    #[test]
    fn rejected_page_parent_still_inherits_as_a_leaf() {
        let pdf = damaged_pdf([
            "<< /Type /Pages /MediaBox [0 0 32 18] >>",
            "<< /Contents [] /Parent 5 0 R >>",
            "<< /Contents [] /Parent 5 0 R >>",
            "<< /Type /Page /Contents [] /Parent 2 0 R >>",
        ]);
        let sizes: Vec<_> = pdf.pages().iter().map(Page::render_dimensions).collect();
        let default_size = (A4.width() as f32, A4.height() as f32);
        assert_eq!(sizes, [default_size, default_size, (32., 18.)]);
    }

    #[test]
    fn recovered_pages_share_a_deep_parent_chain() {
        let depth = 1024;
        let mut objects = vec![String::from("<< /MediaBox [0 0 32 18] >>")];
        objects.extend((2..depth + 2).map(|id| format!("<< /Parent {id} 0 R >>")));
        objects.extend(core::iter::repeat_n(
            format!("<< /Contents [] /Parent {} 0 R >>", depth + 2),
            depth,
        ));
        let pdf = damaged_pdf(objects);
        assert_eq!(pdf.pages().len(), depth);
        assert!(
            pdf.pages()
                .iter()
                .all(|page| page.render_dimensions() == (32., 18.))
        );
    }
}
