//! Reading the pages of a PDF document.

use crate::content::{TypedIter, UntypedIter};
use crate::object::Array;
use crate::object::Dict;
use crate::object::Name;
use crate::object::Rect;
use crate::object::Stream;
use crate::object::dict::keys::*;
use crate::object::{MaybeRef, ObjRef};
use crate::object::{Object, ObjectLike};
use crate::reader::ReaderContext;
use crate::util::FloatExt;
use crate::xref::XRef;
use log::warn;
use std::ops::Deref;
use std::sync::OnceLock;

/// Attributes that can be inherited.
#[derive(Debug, Clone)]
struct PagesContext {
    media_box: Option<Rect>,
    crop_box: Option<Rect>,
    rotate: Option<u32>,
}

impl PagesContext {
    fn new() -> Self {
        Self {
            media_box: None,
            crop_box: None,
            rotate: None,
        }
    }
}

/// A structure holding the pages of a PDF document.
pub struct Pages<'a> {
    pages: Vec<Page<'a>>,
    xref: &'a XRef,
}

impl<'a> Pages<'a> {
    /// Create a new `Pages` object.
    pub(crate) fn new(
        pages_dict: &Dict<'a>,
        ctx: &ReaderContext<'a>,
        xref: &'a XRef,
    ) -> Option<Pages<'a>> {
        let mut pages = vec![];
        let pages_ctx = PagesContext::new();
        resolve_pages(
            pages_dict,
            &mut pages,
            pages_ctx,
            Resources::new(Dict::empty(), None, ctx),
        )?;

        Some(Self { pages, xref })
    }

    /// Create a new `Pages` object by bruteforce-searching.
    ///
    /// Of course this could result in the order of pages being messed up, but
    /// this is still better than nothing.
    pub(crate) fn new_brute_force(ctx: &ReaderContext<'a>, xref: &'a XRef) -> Option<Pages<'a>> {
        let mut pages = vec![];

        for object in xref.objects() {
            if let Some(dict) = object.into_dict()
                && let Some(page) = Page::new(
                    &dict,
                    &PagesContext::new(),
                    Resources::new(Dict::empty(), None, ctx),
                )
            {
                pages.push(page);
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

fn resolve_pages<'a>(
    pages_dict: &Dict<'a>,
    entries: &mut Vec<Page<'a>>,
    mut ctx: PagesContext,
    resources: Resources<'a>,
) -> Option<()> {
    if let Some(media_box) = pages_dict.get::<Rect>(MEDIA_BOX) {
        ctx.media_box = Some(media_box);
    }

    if let Some(crop_box) = pages_dict.get::<Rect>(CROP_BOX) {
        ctx.crop_box = Some(crop_box);
    }

    if let Some(rotate) = pages_dict.get::<u32>(ROTATE) {
        ctx.rotate = Some(rotate);
    }

    let resources = Resources::from_parent(
        pages_dict.get::<Dict>(RESOURCES).unwrap_or_default(),
        resources.clone(),
    );

    let kids = pages_dict.get::<Array<'a>>(KIDS)?;

    for dict in kids.iter::<Dict>() {
        match dict.get::<Name>(TYPE).as_deref() {
            Some(PAGES) => {
                resolve_pages(&dict, entries, ctx.clone(), resources.clone());
            }
            // Let's be lenient and assume it's a `Page` in case it's `None` or something else
            // (see corpus test case 0083781).
            _ => {
                if let Some(page) = Page::new(&dict, &ctx, resources.clone()) {
                    entries.push(page);
                }
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
    page_streams: OnceLock<Option<Vec<u8>>>,
    resources: Resources<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> Page<'a> {
    fn new(dict: &Dict<'a>, ctx: &PagesContext, resources: Resources<'a>) -> Option<Page<'a>> {
        if !dict.contains_key(CONTENTS) {
            return None;
        }

        let media_box = dict.get::<Rect>(MEDIA_BOX).or(ctx.media_box).unwrap_or(A4);

        let crop_box = dict
            .get::<Rect>(CROP_BOX)
            .or(ctx.crop_box)
            .unwrap_or(media_box);

        let rotation = match dict.get::<u32>(ROTATE).or(ctx.rotate).unwrap_or(0) % 360 {
            0 => Rotation::None,
            90 => Rotation::Horizontal,
            180 => Rotation::Flipped,
            270 => Rotation::FlippedHorizontal,
            _ => Rotation::None,
        };

        let ctx = resources.ctx.clone();
        let resources =
            Resources::from_parent(dict.get::<Dict>(RESOURCES).unwrap_or_default(), resources);

        Some(Self {
            inner: dict.clone(),
            media_box,
            crop_box,
            rotation,
            page_streams: OnceLock::new(),
            resources,
            ctx,
        })
    }

    fn operations_impl(&self) -> Option<UntypedIter<'_>> {
        let stream = self.page_stream()?;
        let iter = UntypedIter::new(stream);

        Some(iter)
    }

    /// Return the decoded content stream of the page.
    pub fn page_stream(&self) -> Option<&[u8]> {
        let convert_single = |s: Stream| {
            let data = s.decoded().ok()?;
            Some(data.to_vec())
        };

        self.page_streams
            .get_or_init(|| {
                if let Some(stream) = self.inner.get::<Stream>(CONTENTS) {
                    convert_single(stream)
                } else if let Some(array) = self.inner.get::<Array>(CONTENTS) {
                    let streams = array.iter::<Stream>().flat_map(convert_single);

                    let mut collected = vec![];

                    for stream in streams {
                        collected.extend(stream);
                        // Streams must have at least one whitespace in-between.
                        collected.push(b' ')
                    }

                    Some(collected)
                } else {
                    warn!("contents entry of page was neither stream nor array of streams");

                    None
                }
            })
            .as_ref()
            .map(|d| d.as_slice())
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

    /// Return the base dimensions of the page (same as `intersected_crop_box`, but with special
    /// handling applied for zero-area pages).
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

    /// Return the with and height of the page that should be assumed when rendering the page.
    ///
    /// Depending on the document, it is either based on the media box or the crop box
    /// of the page. In addition to that, it also takes the rotation of the page into account.
    pub fn render_dimensions(&self) -> (f32, f32) {
        let (mut base_width, mut base_height) = self.base_dimensions();

        if matches!(
            self.rotation(),
            Rotation::Horizontal | Rotation::FlippedHorizontal
        ) {
            std::mem::swap(&mut base_width, &mut base_height);
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
        self.ctx.xref
    }

    /// Return a typed iterator over the operators of the page's content stream.
    pub fn typed_operations(&self) -> TypedIter<'_> {
        TypedIter::from_untyped(self.operations())
    }
}

/// A structure keeping track of the resources of a page.
#[derive(Clone, Debug)]
pub struct Resources<'a> {
    parent: Option<Box<Resources<'a>>>,
    ctx: ReaderContext<'a>,
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
    /// Create a new `Resources` object from a dictionary with a parent.
    pub fn from_parent(resources: Dict<'a>, parent: Resources<'a>) -> Resources<'a> {
        let ctx = parent.ctx.clone();

        Self::new(resources, Some(parent), &ctx)
    }

    /// Create a new `Resources` object.
    pub(crate) fn new(
        resources: Dict<'a>,
        parent: Option<Resources<'a>>,
        ctx: &ReaderContext<'a>,
    ) -> Resources<'a> {
        let ext_g_states = resources.get::<Dict>(EXT_G_STATE).unwrap_or_default();
        let fonts = resources.get::<Dict>(FONT).unwrap_or_default();
        let color_spaces = resources.get::<Dict>(COLORSPACE).unwrap_or_default();
        let x_objects = resources.get::<Dict>(XOBJECT).unwrap_or_default();
        let patterns = resources.get::<Dict>(PATTERN).unwrap_or_default();
        let shadings = resources.get::<Dict>(SHADING).unwrap_or_default();
        let properties = resources.get::<Dict>(PROPERTIES).unwrap_or_default();

        let parent = parent.map(Box::new);

        Self {
            parent,
            ext_g_states,
            fonts,
            color_spaces,
            properties,
            x_objects,
            patterns,
            shadings,
            ctx: ctx.clone(),
        }
    }

    /// Resolve an object reference to an object.
    #[allow(private_bounds)]
    pub fn resolve_ref<T: ObjectLike<'a>>(&self, ref_: ObjRef) -> Option<T> {
        self.ctx.xref.get_with(ref_.into(), &self.ctx)
    }

    fn get_resource<T: ObjectLike<'a>, U>(
        &self,
        name: Name,
        dict: &Dict<'a>,
        mut cache: impl FnMut(ObjRef) -> Option<U>,
        mut resolve: impl FnMut(T) -> Option<U>,
    ) -> Option<U> {
        // TODO: Cache non-ref resources as well

        match dict.get_raw::<T>(name.deref())? {
            MaybeRef::Ref(ref_) => cache(ref_).or_else(|| {
                self.ctx
                    .xref
                    .get_with::<T>(ref_.into(), &self.ctx)
                    .and_then(&mut resolve)
            }),
            MaybeRef::NotRef(i) => resolve(i),
        }
    }

    /// Get the parent in the resource, chain, if available.
    pub fn parent(&self) -> Option<&Resources<'a>> {
        self.parent.as_deref()
    }

    // TODO: Refactor caching mechanism

    /// Get an external graphics state by name.
    pub fn get_ext_g_state<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Dict<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Dict, U>(name.clone(), &self.ext_g_states, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_ext_g_state::<U>(name, cache, resolve))
            })
    }

    /// Get a color space by name.
    pub fn get_color_space<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Object<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Object, U>(name.clone(), &self.color_spaces, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_color_space::<U>(name, cache, resolve))
            })
    }

    /// Get a font by name.
    pub fn get_font<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Dict<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Dict, U>(name.clone(), &self.fonts, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_font::<U>(name, cache, resolve))
            })
    }

    /// Get a pattern by name.
    pub fn get_pattern<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Object<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Object, U>(name.clone(), &self.patterns, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_pattern::<U>(name, cache, resolve))
            })
    }

    /// Get an x object by name.
    pub fn get_x_object<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Stream<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Stream, U>(name.clone(), &self.x_objects, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_x_object::<U>(name, cache, resolve))
            })
    }

    /// Get a shading by name.
    pub fn get_shading<U>(
        &self,
        name: Name,
        mut cache: Box<dyn FnMut(ObjRef) -> Option<U> + '_>,
        mut resolve: Box<dyn FnMut(Object<'a>) -> Option<U> + '_>,
    ) -> Option<U> {
        self.get_resource::<Object, U>(name.clone(), &self.shadings, &mut cache, &mut resolve)
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|p| p.get_shading::<U>(name, cache, resolve))
            })
    }
}

// <https://github.com/apache/pdfbox/blob/a53a70db16ea3133994120bcf1e216b9e760c05b/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/common/PDRectangle.java#L38>
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
    use std::ops::Deref;
    use std::sync::Arc;

    pub(crate) struct CachedPages {
        pages: Pages<'static>,
        // NOTE: `pages` references the data in `xref`, so it's important that `xref`
        // appears after `pages` in the struct definition to ensure correct drop order.
        _xref: Arc<XRef>,
    }

    impl CachedPages {
        pub(crate) fn new(xref: Arc<XRef>) -> Option<Self> {
            // SAFETY:
            // - The XRef's location is stable in memory:
            //   - We wrapped it in a `Arc`, which implements `StableDeref`.
            //   - The struct owns the `Arc`, ensuring that the inner value is not dropped during the whole
            //     duration.
            // - The internal 'static lifetime is not leaked because its rewritten
            //   to the self-lifetime in `pages()`.
            let xref_reference: &'static XRef = unsafe { std::mem::transmute(xref.deref()) };

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
