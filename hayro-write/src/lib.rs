mod primitive;

use crate::primitive::{WriteDirect, WriteIndirect};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{
    COLORSPACE, CONTENTS, EXT_G_STATE, FILTER, FONT, GROUP, PATTERN, PROPERTIES, RESOURCES,
    SHADING, XOBJECT,
};
use hayro_syntax::object::{MaybeRef, ObjRef};
use hayro_syntax::page::{Resources, Rotation};
use log::warn;
use pdf_writer::{Chunk, Content, Filter, Finish, Name, Obj, Rect, Ref};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Deref;
use std::ops::DerefMut;

pub use hayro_syntax::{Pdf, PdfData, PdfVersion};

#[derive(Copy, Clone, Debug)]
pub enum ExtractionQueryType {
    XObject,
    Page,
}

#[derive(Copy, Clone, Debug)]
pub struct ExtractionQuery {
    query_type: ExtractionQueryType,
    page_index: usize,
}

impl ExtractionQuery {
    pub fn new_page(page_index: usize) -> Self {
        Self {
            query_type: ExtractionQueryType::Page,
            page_index,
        }
    }

    pub fn new_xobject(page_index: usize) -> Self {
        Self {
            query_type: ExtractionQueryType::XObject,
            page_index,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ExtractionError {
    LoadPdfError,
    InvalidPageIndex(usize),
    InvalidPdf,
}

pub struct ExtractionResult {
    pub chunk: Chunk,
    pub root_refs: Vec<Result<Ref, ExtractionError>>,
    pub page_tree_parent_ref: Ref,
}

struct ExtractionContext<'a> {
    chunks: Vec<Chunk>,
    visited_objects: HashSet<ObjRef>,
    to_visit_refs: Vec<ObjRef>,
    root_refs: Vec<Result<Ref, ExtractionError>>,
    new_ref: Box<dyn FnMut() -> Ref + 'a>,
    ref_map: HashMap<ObjRef, Ref>,
    cached_content_streams: HashMap<usize, Ref>,
    page_tree_parent_ref: Ref,
}

impl<'a> ExtractionContext<'a> {
    pub fn new(mut new_ref: Box<dyn FnMut() -> Ref + 'a>) -> Self {
        let page_tree_parent_ref = new_ref();
        Self {
            chunks: vec![],
            visited_objects: HashSet::new(),
            to_visit_refs: Vec::new(),
            new_ref,
            ref_map: HashMap::new(),
            cached_content_streams: HashMap::new(),
            root_refs: Vec::new(),
            page_tree_parent_ref,
        }
    }

    pub fn map_ref(&mut self, ref_: ObjRef) -> pdf_writer::Ref {
        if let Some(ref_) = self.ref_map.get(&ref_) {
            *ref_
        } else {
            let new_ref = self.new_ref();
            self.ref_map.insert(ref_, new_ref);

            new_ref
        }
    }

    pub fn new_ref(&mut self) -> pdf_writer::Ref {
        (self.new_ref)()
    }
}

pub fn extract<'a>(
    pdf: &Pdf,
    new_ref: Box<dyn FnMut() -> Ref + 'a>,
    queries: &[ExtractionQuery],
) -> Result<ExtractionResult, ExtractionError> {
    let pages = pdf.pages();
    let mut ctx = ExtractionContext::new(new_ref);

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

    write_dependencies(pdf, &mut ctx);

    let mut global_chunk = Chunk::new();

    for chunk in &ctx.chunks {
        global_chunk.extend(&chunk)
    }

    Ok(ExtractionResult {
        chunk: global_chunk,
        root_refs: ctx.root_refs,
        page_tree_parent_ref: ctx.page_tree_parent_ref,
    })
}

fn write_dependencies(pdf: &Pdf, ctx: &mut ExtractionContext) {
    while let Some(ref_) = ctx.to_visit_refs.pop() {
        if ctx.visited_objects.contains(&ref_) {
            continue;
        }

        let mut chunk = Chunk::new();
        if let Some(object) = pdf.xref().get::<Object>(ref_.into()) {
            let new_ref = ctx.map_ref(ref_);
            object.write_indirect(&mut chunk, new_ref, ctx);
            ctx.chunks.push(chunk);

            ctx.visited_objects.insert(ref_);
        } else {
            warn!("failed to extract object with ref: {:?}", ref_);
        }
    }
}

// Only used for testing.
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

    let extracted = extract(&hayro_pdf, Box::new(|| next_ref.bump()), &requests).unwrap();
    pdf.catalog(catalog_id)
        .pages(extracted.page_tree_parent_ref);
    let count = extracted.root_refs.len();
    pdf.pages(extracted.page_tree_parent_ref)
        .kids(extracted.root_refs.iter().map(|r| r.unwrap()))
        .count(count as i32);
    pdf.extend(&extracted.chunk);

    pdf.finish()
}

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

    let extracted = extract(&hayro_pdf, Box::new(|| next_ref.bump()), &requests).unwrap();

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
    page: &hayro_syntax::page::Page,
    page_ref: Ref,
    page_idx: usize,
    ctx: &mut ExtractionContext,
) -> Result<(), ExtractionError> {
    let mut chunk = Chunk::new();
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

    if let Some(group) = raw_dict.get_raw::<Object>(GROUP) {
        group.write_direct(pdf_page.insert(pdf_writer::Name(GROUP)), ctx);
    }

    serialize_resources(page.resources(), ctx, &mut pdf_page);

    pdf_page.finish();

    ctx.chunks.push(chunk);

    Ok(())
}

fn write_xobject(
    page: &hayro_syntax::page::Page,
    xobj_ref: Ref,
    ctx: &mut ExtractionContext,
) -> Result<(), ExtractionError> {
    let mut chunk = Chunk::new();
    let encoded_stream = deflate_encode(page.page_stream().unwrap_or(b""));
    let mut x_object = chunk.form_xobject(xobj_ref, &encoded_stream);
    x_object.deref_mut().filter(Filter::FlateDecode);

    let bbox = page.crop_box();
    let initial_transform = page.initial_transform(false);

    x_object.bbox(pdf_writer::Rect::new(
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
    resources: &Resources,
    ctx: &mut ExtractionContext,
    writer: &mut impl ResourcesExt,
) {
    let ext_g_states = collect_resources(
        &resources,
        |r| r.ext_g_states.clone(),
        hayro_syntax::object::Name::new(EXT_G_STATE),
    );
    let shadings = collect_resources(
        &resources,
        |r| r.shadings.clone(),
        hayro_syntax::object::Name::new(SHADING),
    );
    let patterns = collect_resources(
        &resources,
        |r| r.patterns.clone(),
        hayro_syntax::object::Name::new(PATTERN),
    );
    let x_objects = collect_resources(
        &resources,
        |r| r.x_objects.clone(),
        hayro_syntax::object::Name::new(XOBJECT),
    );
    let color_spaces = collect_resources(
        &resources,
        |r| r.color_spaces.clone(),
        hayro_syntax::object::Name::new(COLORSPACE),
    );
    let fonts = collect_resources(
        &resources,
        |r| r.fonts.clone(),
        hayro_syntax::object::Name::new(FONT),
    );
    let properties = collect_resources(
        &resources,
        |r| r.properties.clone(),
        hayro_syntax::object::Name::new(PROPERTIES),
    );

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
    mut get_dict: impl FnMut(&Resources<'a>) -> Dict<'a> + Clone,
    name: hayro_syntax::object::Name<'a>,
) -> BTreeMap<hayro_syntax::object::Name<'a>, MaybeRef<Object<'a>>> {
    let mut map = BTreeMap::new();
    collect_resources_inner(resources, get_dict, name, &mut map);
    map
}

fn collect_resources_inner<'a>(
    resources: &Resources<'a>,
    mut get_dict: impl FnMut(&Resources<'a>) -> Dict<'a> + Clone,
    name: hayro_syntax::object::Name<'a>,
    map: &mut BTreeMap<hayro_syntax::object::Name<'a>, MaybeRef<Object<'a>>>,
) {
    // Process parents first, so that duplicates get overridden by the current dictionary.
    if let Some(parent) = resources.parent() {
        collect_resources_inner(parent, get_dict.clone(), name, map);
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

fn convert_rect(hy_rect: &hayro_syntax::object::Rect) -> pdf_writer::Rect {
    pdf_writer::Rect::new(
        hy_rect.x0 as f32,
        hy_rect.y0 as f32,
        hy_rect.x1 as f32,
        hy_rect.y1 as f32,
    )
}

trait ResourcesExt {
    fn resources(&mut self) -> pdf_writer::writers::Resources;
}

impl ResourcesExt for pdf_writer::writers::Page<'_> {
    fn resources(&mut self) -> pdf_writer::writers::Resources {
        Self::resources(self)
    }
}

impl ResourcesExt for pdf_writer::writers::FormXObject<'_> {
    fn resources(&mut self) -> pdf_writer::writers::Resources {
        Self::resources(self)
    }
}
