/*!
A crate for converting PDF pages into either `XObjects` or a new page via [`pdf-writer`](https://docs.rs/pdf-writer/).

This is an internal crate and not meant for external use. Therefore, it's not very
well-documented.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod primitive;

use crate::primitive::{WriteDirect, WriteIndirect};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::keys::{
    COLORSPACE, EXT_G_STATE, FONT, GROUP, PATTERN, PROPERTIES, SHADING, XOBJECT,
};
use hayro_syntax::object::{MaybeRef, ObjRef};
use hayro_syntax::page::{Resources, Rotation};
use kurbo::Affine;
use log::warn;
use pdf_writer::{Chunk, Content, Filter, Finish, Name, Rect, Ref};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Deref;
use std::ops::DerefMut;

pub use hayro_syntax::page::{Page, Pages};
pub use hayro_syntax::{LoadPdfError, Pdf, PdfData, PdfVersion};

/// Apply the extraction queries to the given PDF and return the results.
pub fn extract<'a>(
    pdf: &Pdf,
    new_ref: Box<dyn FnMut() -> Ref + 'a>,
    queries: &[ExtractionQuery],
) -> Result<ExtractionResult, ExtractionError> {
    let pages = pdf.pages();
    let mut ctx = ExtractionContext::new(new_ref, pdf);

    for query in queries {
        let page = pages
            .get(query.page_index)
            .ok_or(ExtractionError::InvalidPageIndex(query.page_index))?;

        let root_ref = ctx.new_ref();

        let res = match query.query_type {
            ExtractionQueryType::XObject => write_xobject(page, root_ref, &mut ctx),
            ExtractionQueryType::Page => write_page(page, root_ref, query.page_index, &mut ctx),
        };

        ctx.root_refs.push(res.map(|_| root_ref));
    }

    // Now we have shallowly extracted all pages, now go through all dependencies until there aren't
    // any anymore.
    write_dependencies(pdf, &mut ctx);

    let mut global_chunk = Chunk::new();

    for chunk in &ctx.chunks {
        global_chunk.extend(chunk);
    }

    Ok(ExtractionResult {
        chunk: global_chunk,
        root_refs: ctx.root_refs,
        page_tree_parent_ref: ctx.page_tree_parent_ref,
    })
}

/// A type of extraction query, indicating as what kind of
/// object you want to extract the page.
#[derive(Copy, Clone, Debug)]
pub enum ExtractionQueryType {
    /// Extract the page as an `XObject`.
    XObject,
    /// Extract the page as a new page.
    Page,
}

/// An extraction query.
#[derive(Copy, Clone, Debug)]
pub struct ExtractionQuery {
    query_type: ExtractionQueryType,
    page_index: usize,
}

impl ExtractionQuery {
    /// Create a new page extraction query with the given page index.
    pub fn new_page(page_index: usize) -> Self {
        Self {
            query_type: ExtractionQueryType::Page,
            page_index,
        }
    }

    /// Create a new `XObject` extraction query with the given page index.
    pub fn new_xobject(page_index: usize) -> Self {
        Self {
            query_type: ExtractionQueryType::XObject,
            page_index,
        }
    }
}

/// An error that occurred during page extraction.
#[derive(Debug, Copy, Clone)]
pub enum ExtractionError {
    /// An invalid page index was given.
    InvalidPageIndex(usize),
}

/// The result of an extraction.
pub struct ExtractionResult {
    /// The chunk containing all objects as well as their dependencies.
    pub chunk: Chunk,
    /// The root references of the pages/XObject, one for each extraction query.
    pub root_refs: Vec<Result<Ref, ExtractionError>>,
    /// The reference to the page tree parent that was generated.
    pub page_tree_parent_ref: Ref,
}

struct ExtractionContext<'a> {
    chunks: Vec<Chunk>,
    visited_objects: HashSet<ObjRef>,
    to_visit_refs: Vec<ObjRef>,
    valid_ref_cache: HashMap<ObjRef, bool>,
    root_refs: Vec<Result<Ref, ExtractionError>>,
    pdf: &'a Pdf,
    new_ref: Box<dyn FnMut() -> Ref + 'a>,
    ref_map: HashMap<ObjRef, Ref>,
    cached_content_streams: HashMap<usize, Ref>,
    page_tree_parent_ref: Ref,
}

impl<'a> ExtractionContext<'a> {
    fn new(mut new_ref: Box<dyn FnMut() -> Ref + 'a>, pdf: &'a Pdf) -> Self {
        let page_tree_parent_ref = new_ref();
        Self {
            chunks: vec![],
            visited_objects: HashSet::new(),
            to_visit_refs: Vec::new(),
            valid_ref_cache: HashMap::new(),
            pdf,
            new_ref,
            ref_map: HashMap::new(),
            cached_content_streams: HashMap::new(),
            root_refs: Vec::new(),
            page_tree_parent_ref,
        }
    }

    pub(crate) fn map_ref(&mut self, ref_: ObjRef) -> Ref {
        if let Some(ref_) = self.ref_map.get(&ref_) {
            *ref_
        } else {
            let new_ref = self.new_ref();
            self.ref_map.insert(ref_, new_ref);

            new_ref
        }
    }

    pub(crate) fn new_ref(&mut self) -> Ref {
        (self.new_ref)()
    }
}

fn write_dependencies(pdf: &Pdf, ctx: &mut ExtractionContext<'_>) {
    while let Some(ref_) = ctx.to_visit_refs.pop() {
        // Don't visit objects twice!
        if ctx.visited_objects.contains(&ref_) {
            continue;
        }

        let mut chunk = Chunk::new();
        if let Some(object) = pdf.xref().get::<Object<'_>>(ref_.into()) {
            let new_ref = ctx.map_ref(ref_);
            object.write_indirect(&mut chunk, new_ref, ctx);
            ctx.chunks.push(chunk);

            ctx.visited_objects.insert(ref_);
        } else {
            warn!("failed to extract object with ref: {ref_:?}");
        }
    }
}

/// Extract the given pages from the PDF and resave them as a new PDF. This function shouldn't be
/// used directly and only exists for test purposes.
#[doc(hidden)]
pub fn extract_pages_to_pdf(hayro_pdf: &Pdf, page_indices: &[usize]) -> Vec<u8> {
    let mut pdf = pdf_writer::Pdf::new();
    let mut next_ref = Ref::new(1);
    let requests = page_indices
        .iter()
        .map(|i| ExtractionQuery {
            query_type: ExtractionQueryType::Page,
            page_index: *i,
        })
        .collect::<Vec<_>>();

    let catalog_id = next_ref.bump();

    let extracted = extract(hayro_pdf, Box::new(|| next_ref.bump()), &requests).unwrap();
    pdf.catalog(catalog_id)
        .pages(extracted.page_tree_parent_ref);
    let count = extracted.root_refs.len();
    pdf.pages(extracted.page_tree_parent_ref)
        .kids(extracted.root_refs.iter().map(|r| r.unwrap()))
        .count(count as i32);
    pdf.extend(&extracted.chunk);

    pdf.finish()
}

/// Extract the given pages as XObjects from the PDF and resave them as a new PDF.
/// This function shouldn't be used directly and only exists for test purposes.
#[doc(hidden)]
pub fn extract_pages_as_xobject_to_pdf(hayro_pdf: &Pdf, page_indices: &[usize]) -> Vec<u8> {
    let hayro_pages = hayro_pdf.pages();
    let page_list = hayro_pages.as_ref();

    let mut pdf = pdf_writer::Pdf::new();
    let mut next_ref = Ref::new(1);

    let catalog_id = next_ref.bump();
    let requests = page_indices
        .iter()
        .map(|i| ExtractionQuery {
            query_type: ExtractionQueryType::XObject,
            page_index: *i,
        })
        .collect::<Vec<_>>();

    let extracted = extract(hayro_pdf, Box::new(|| next_ref.bump()), &requests).unwrap();

    pdf.catalog(catalog_id)
        .pages(extracted.page_tree_parent_ref);
    let mut page_refs = vec![];

    for (x_object_ref, page_idx) in extracted.root_refs.iter().zip(page_indices) {
        let page = &page_list[*page_idx];
        let render_dimensions = page.render_dimensions();

        let mut content = Content::new();
        content.x_object(Name(b"O1"));

        let finished = content.finish();

        let page_id = next_ref.bump();
        let stream_id = next_ref.bump();
        page_refs.push(page_id);

        let mut page = pdf.page(page_id);
        page.resources()
            .x_objects()
            .pair(Name(b"O1"), x_object_ref.unwrap());
        page.media_box(Rect::new(
            0.0,
            0.0,
            render_dimensions.0,
            render_dimensions.1,
        ));
        page.parent(extracted.page_tree_parent_ref);
        page.contents(stream_id);
        page.finish();

        pdf.stream(stream_id, finished.as_slice());
    }

    let count = extracted.root_refs.len();
    pdf.pages(extracted.page_tree_parent_ref)
        .kids(page_refs)
        .count(count as i32);
    pdf.extend(&extracted.chunk);

    pdf.finish()
}

fn write_page(
    page: &Page<'_>,
    page_ref: Ref,
    page_idx: usize,
    ctx: &mut ExtractionContext<'_>,
) -> Result<(), ExtractionError> {
    let mut chunk = Chunk::new();
    // Note: We can cache content stream references, but _not_ the page references themselves.
    // Acrobat for some reason doesn't like duplicate page references in the page tree.
    let stream_ref = if let Some(cached) = ctx.cached_content_streams.get(&page_idx) {
        *cached
    } else {
        let stream_ref = ctx.new_ref();

        chunk
            .stream(
                stream_ref,
                &deflate_encode(page.page_stream().unwrap_or(b"")),
            )
            .filter(Filter::FlateDecode);
        ctx.cached_content_streams.insert(page_idx, stream_ref);

        stream_ref
    };

    let mut pdf_page = chunk.page(page_ref);

    pdf_page
        .media_box(convert_rect(&page.media_box()))
        .crop_box(convert_rect(&page.crop_box()))
        .rotate(match page.rotation() {
            Rotation::None => 0,
            Rotation::Horizontal => 90,
            Rotation::Flipped => 180,
            Rotation::FlippedHorizontal => 270,
        })
        .parent(ctx.page_tree_parent_ref)
        .contents(stream_ref);

    let raw_dict = page.raw();

    if let Some(group) = raw_dict.get_raw::<Object<'_>>(GROUP) {
        group.write_direct(pdf_page.insert(Name(GROUP)), ctx);
    }

    serialize_resources(page.resources(), ctx, &mut pdf_page);

    pdf_page.finish();

    ctx.chunks.push(chunk);

    Ok(())
}

fn write_xobject(
    page: &Page<'_>,
    xobj_ref: Ref,
    ctx: &mut ExtractionContext<'_>,
) -> Result<(), ExtractionError> {
    let mut chunk = Chunk::new();
    let encoded_stream = deflate_encode(page.page_stream().unwrap_or(b""));
    let mut x_object = chunk.form_xobject(xobj_ref, &encoded_stream);
    x_object.deref_mut().filter(Filter::FlateDecode);

    let bbox = page.crop_box();
    let initial_transform = page.initial_transform(false);

    x_object.bbox(Rect::new(
        bbox.x0 as f32,
        bbox.y0 as f32,
        bbox.x1 as f32,
        bbox.y1 as f32,
    ));

    let i = initial_transform.as_coeffs();
    x_object.matrix([
        i[0] as f32,
        i[1] as f32,
        i[2] as f32,
        i[3] as f32,
        i[4] as f32,
        i[5] as f32,
    ]);

    serialize_resources(page.resources(), ctx, &mut x_object);

    x_object.finish();
    ctx.chunks.push(chunk);

    Ok(())
}

fn serialize_resources(
    resources: &Resources<'_>,
    ctx: &mut ExtractionContext<'_>,
    writer: &mut impl ResourcesExt,
) {
    let ext_g_states = collect_resources(resources, |r| r.ext_g_states.clone());
    let shadings = collect_resources(resources, |r| r.shadings.clone());
    let patterns = collect_resources(resources, |r| r.patterns.clone());
    let x_objects = collect_resources(resources, |r| r.x_objects.clone());
    let color_spaces = collect_resources(resources, |r| r.color_spaces.clone());
    let fonts = collect_resources(resources, |r| r.fonts.clone());
    let properties = collect_resources(resources, |r| r.properties.clone());

    if !(ext_g_states.is_empty()
        && shadings.is_empty()
        && patterns.is_empty()
        && x_objects.is_empty()
        && color_spaces.is_empty()
        && properties.is_empty()
        && fonts.is_empty())
    {
        let mut resources = writer.resources();

        macro_rules! write {
            ($name:ident, $key:expr) => {
                if !$name.is_empty() {
                    let mut dict = resources.insert(Name($key)).dict();

                    for (name, obj) in $name {
                        obj.write_direct(dict.insert(Name(name.deref())), ctx);
                    }
                }
            };
        }

        write!(ext_g_states, EXT_G_STATE);
        write!(shadings, SHADING);
        write!(patterns, PATTERN);
        write!(x_objects, XOBJECT);
        write!(color_spaces, COLORSPACE);
        write!(fonts, FONT);
        write!(properties, PROPERTIES);
    }
}

fn collect_resources<'a>(
    resources: &Resources<'a>,
    get_dict: impl FnMut(&Resources<'a>) -> Dict<'a> + Clone,
) -> BTreeMap<hayro_syntax::object::Name<'a>, MaybeRef<Object<'a>>> {
    let mut map = BTreeMap::new();
    collect_resources_inner(resources, get_dict, &mut map);
    map
}

fn collect_resources_inner<'a>(
    resources: &Resources<'a>,
    mut get_dict: impl FnMut(&Resources<'a>) -> Dict<'a> + Clone,
    map: &mut BTreeMap<hayro_syntax::object::Name<'a>, MaybeRef<Object<'a>>>,
) {
    // Process parents first, so that duplicates get overridden by the current dictionary.
    // Since for inheritance, the current dictionary always has priority over entries in the
    // parent dictionary.
    if let Some(parent) = resources.parent() {
        collect_resources_inner(parent, get_dict.clone(), map);
    }

    let dict = get_dict(resources);

    for (name, object) in dict.entries() {
        map.insert(name, object);
    }
}

pub(crate) fn deflate_encode(data: &[u8]) -> Vec<u8> {
    use std::io::Write;

    const COMPRESSION_LEVEL: u8 = 6;
    let mut e = ZlibEncoder::new(Vec::new(), Compression::new(COMPRESSION_LEVEL as u32));
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn convert_rect(hy_rect: &hayro_syntax::object::Rect) -> Rect {
    Rect::new(
        hy_rect.x0 as f32,
        hy_rect.y0 as f32,
        hy_rect.x1 as f32,
        hy_rect.y1 as f32,
    )
}

trait ResourcesExt {
    fn resources(&mut self) -> pdf_writer::writers::Resources<'_>;
}

impl ResourcesExt for pdf_writer::writers::Page<'_> {
    fn resources(&mut self) -> pdf_writer::writers::Resources<'_> {
        Self::resources(self)
    }
}

impl ResourcesExt for pdf_writer::writers::FormXObject<'_> {
    fn resources(&mut self) -> pdf_writer::writers::Resources<'_> {
        Self::resources(self)
    }
}

// Note: Keep in sync with `hayro-interpret`.
trait PageExt {
    /// Return the initial transform that should be applied when rendering. This accounts for a
    /// number of factors, such as the mismatch between PDF's y-up and most renderers' y-down
    /// coordinate system, the rotation of the page and the offset of the crop box.
    fn initial_transform(&self, invert_y: bool) -> Affine;
}

impl PageExt for Page<'_> {
    fn initial_transform(&self, invert_y: bool) -> Affine {
        let crop_box = self.intersected_crop_box();
        let (_, base_height) = self.base_dimensions();
        let (width, height) = self.render_dimensions();

        let horizontal_t =
            Affine::rotate(90.0_f64.to_radians()) * Affine::translate((0.0, -width as f64));
        let flipped_horizontal_t =
            Affine::translate((0.0, height as f64)) * Affine::rotate(-90.0_f64.to_radians());

        let rotation_transform = match self.rotation() {
            Rotation::None => Affine::IDENTITY,
            Rotation::Horizontal => {
                if invert_y {
                    horizontal_t
                } else {
                    flipped_horizontal_t
                }
            }
            Rotation::Flipped => {
                Affine::scale(-1.0) * Affine::translate((-width as f64, -height as f64))
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
            Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, base_height as f64])
        } else {
            Affine::IDENTITY
        };

        rotation_transform * inversion_transform * Affine::translate((-crop_box.x0, -crop_box.y0))
    }
}
