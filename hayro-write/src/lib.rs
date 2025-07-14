mod primitive;

use crate::primitive::{WriteDirect, WriteIndirect};
use hayro_syntax::document::page::{Resources, Rotation};
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::{CONTENTS, RESOURCES};
use hayro_syntax::object::r#ref::ObjRef;
use hayro_syntax::pdf::Pdf;
use pdf_writer::{Chunk, Finish, Obj, Ref};
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub enum ExtractionError {
    LoadPdfError,
    InvalidPageIndex(usize, usize),
    InvalidPdf,
}

pub struct ExtractedPages {
    pub chunk: Chunk,
    pub page_refs: Vec<Ref>,
    pub next_ref: Ref,
}

struct ExtractionContext {
    chunks: Vec<Chunk>,
    visited_objects: HashSet<ObjRef>,
    to_visit_refs: Vec<ObjRef>,
    page_refs: Vec<Ref>,
    next_ref: Ref,
    ref_map: HashMap<ObjRef, Ref>,
    page_cache: HashMap<usize, Ref>,
    page_tree_parent_ref: Ref,
}

impl ExtractionContext {
    pub fn new(next_ref: Ref, page_tree_parent_ref: Ref) -> Self {
        Self {
            chunks: vec![],
            visited_objects: HashSet::new(),
            to_visit_refs: Vec::new(),
            next_ref,
            ref_map: HashMap::new(),
            page_cache: HashMap::new(),
            page_refs: Vec::new(),
            page_tree_parent_ref,
        }
    }

    pub fn map_ref(&mut self, ref_: ObjRef) -> pdf_writer::Ref {
        if let Some(ref_) = self.ref_map.get(&ref_) {
            *ref_
        } else {
            let new_ref = self.next_ref.bump();
            self.ref_map.insert(ref_, new_ref);

            new_ref
        }
    }

    pub fn new_ref(&mut self) -> pdf_writer::Ref {
        self.next_ref.bump()
    }
}

pub fn extract_pages(
    pdf: &Pdf,
    next_ref: Ref,
    page_tree_parent_ref: Ref,
    page_indices: &[usize],
) -> Result<ExtractedPages, ExtractionError> {
    let pages = pdf.pages().ok_or(ExtractionError::LoadPdfError)?;
    let mut ctx = ExtractionContext::new(next_ref, page_tree_parent_ref);

    for page_index in page_indices.iter().copied() {
        let page = pages
            .get()
            .get(page_index)
            .ok_or(ExtractionError::InvalidPageIndex(page_index, pages.len()))?;

        if let Some(ref_) = ctx.page_cache.get(&page_index) {
            ctx.page_refs.push(*ref_);
        } else {
            let page_ref = ctx.new_ref();
            ctx.page_cache.insert(page_index, page_ref);
            write_page(page, page_ref, &mut ctx)?;
            ctx.page_refs.push(page_ref);
        }
    }

    while let Some(ref_) = ctx.to_visit_refs.pop() {
        if ctx.visited_objects.contains(&ref_) {
            continue;
        }

        let mut chunk = Chunk::new();
        let object = pdf
            .xref()
            .get::<Object>(ref_.into())
            .ok_or(ExtractionError::InvalidPdf)?;
        let new_ref = ctx.map_ref(ref_);
        object.write_indirect(&mut chunk, new_ref, &mut ctx);
        ctx.chunks.push(chunk);

        ctx.visited_objects.insert(ref_);
    }

    let mut global_chunk = Chunk::new();

    for chunk in &ctx.chunks {
        global_chunk.extend(&chunk)
    }

    Ok(ExtractedPages {
        chunk: global_chunk,
        page_refs: ctx.page_refs,
        next_ref: ctx.next_ref,
    })
}

// Only used for testing.
#[doc(hidden)]
pub fn extract_pages_to_pdf(hayro_pdf: &Pdf, page_indices: &[usize]) -> Vec<u8> {
    let mut pdf = pdf_writer::Pdf::new();
    let mut next_ref = Ref::new(1);

    let catalog_id = next_ref.bump();
    let page_tree_id = next_ref.bump();
    pdf.catalog(catalog_id).pages(page_tree_id);

    let extracted = extract_pages(&hayro_pdf, next_ref, page_tree_id, &page_indices).unwrap();
    let count = extracted.page_refs.len();
    pdf.pages(page_tree_id)
        .kids(extracted.page_refs)
        .count(count as i32);
    pdf.extend(&extracted.chunk);

    pdf.finish()
}

fn write_page(
    page: &hayro_syntax::document::page::Page,
    page_ref: Ref,
    ctx: &mut ExtractionContext,
) -> Result<(), ExtractionError> {
    let mut chunk = Chunk::new();
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
        .parent(ctx.page_tree_parent_ref);

    let raw_dict = page.raw();

    if let Some(contents) = raw_dict.get_raw::<Object>(CONTENTS) {
        contents.write_direct(pdf_page.insert(pdf_writer::Name(CONTENTS)), ctx);
    }

    // TODO: Consider inherited resources as well!
    if let Some(resources) = raw_dict.get_raw::<Dict>(RESOURCES) {
        resources.write_direct(pdf_page.insert(pdf_writer::Name(RESOURCES)), ctx)
    }

    pdf_page.finish();
    ctx.chunks.push(chunk);

    Ok(())
}

fn convert_rect(hy_rect: &hayro_syntax::object::rect::Rect) -> pdf_writer::Rect {
    pdf_writer::Rect::new(
        hy_rect.x0 as f32,
        hy_rect.y0 as f32,
        hy_rect.x1 as f32,
        hy_rect.y1 as f32,
    )
}
