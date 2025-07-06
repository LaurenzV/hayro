mod primitive;

use hayro_syntax as hs;
use hayro_syntax::object::null::Null;
use hayro_syntax::object::r#ref::ObjRef;
use hayro_syntax::pdf::Pdf;
use pdf_writer as pf;
use pdf_writer::{Chunk, Obj, Ref};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub struct ExtractedPages {
    chunk: Chunk,
    page_regs: Vec<ObjRef>,
}

struct ExtractionContext {
    chunks: Vec<Chunk>,
    visited_objects: HashSet<ObjRef>,
    to_visit_refs: Vec<ObjRef>,
    next_ref: Ref,
    ref_map: HashMap<ObjRef, Ref>,
}

impl ExtractionContext {
    pub fn map_ref(&mut self, ref_: ObjRef) -> pdf_writer::Ref {
        if let Some(ref_) = self.ref_map.get(&ref_) {
            *ref_
        } else {
            let new_ref = self.next_ref.bump();
            self.ref_map.insert(ref_, new_ref);

            new_ref
        }
    }
}

//
// pub fn extract_pages(pdf: &Pdf, page_range: &[usize]) -> ExtractedPages {
//     todo!()
// }
//
// fn write_dict(dict: &hs::object::dict::Dict, ctx: &mut ExtractionContext) {
//     for (key, name) in dict.entries() {
//
//     }
// }

fn test() {
    // let mut chunk = Chunk::new();
    // let mut indirect = chunk.indirect(pf::Ref::new(1));
    // let mut d: pf::Dict = indirect.start();
    // let mut r: Ref = d.insert(Name(b"Hi")).start();
}
