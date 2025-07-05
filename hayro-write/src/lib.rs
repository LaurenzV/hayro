mod primitive;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use pdf_writer::{Chunk, Obj, Ref};
use hayro_syntax as hs;
use hayro_syntax::object::r#ref::ObjRef;
use hayro_syntax::pdf::Pdf;
use pdf_writer as pf;
use hayro_syntax::object::null::Null;

pub struct ExtractedPages {
    chunk: Chunk,
    page_regs: Vec<ObjRef>
}

struct ExtractionContext {
    chunks: Vec<Chunk>,
    visited_objects: HashSet<ObjRef>,
    next_ref: Ref,
    ref_map: HashMap<ObjRef, Ref>
}

impl ExtractionContext {
    pub fn map_ref(&mut self, ref_: ObjRef) -> pdf_writer::Ref {
        if let Some(ref_) = self.ref_map.get(&ref_) {
            *ref_
        }   else {
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

fn write_ref(ref_: &ObjRef, obj: Obj, ctx: &mut ExtractionContext) {
    let mapped_ref = ctx.map_ref(ref_.clone());
    obj.primitive(mapped_ref);
}

fn write_number(value: &hayro_syntax::object::number::Number, obj: Obj, ctx: &mut ExtractionContext) {
    let float_num = value.as_f64();
    
    if float_num.fract() == 0.0 {
        obj.primitive(float_num as i32);
    }   else {
        obj.primitive(float_num as f32);
    }
}

fn write_bool(value: bool, obj: Obj, _: &mut ExtractionContext) {
    obj.primitive(value);
}

fn write_null(_: Null, obj: Obj, _: &mut ExtractionContext) {
    obj.primitive(pf::Null);
}

fn test() {
    // let mut chunk = Chunk::new();
    // let mut indirect = chunk.indirect(pf::Ref::new(1));
    // let mut d: pf::Dict = indirect.start();
    // let mut r: Ref = d.insert(Name(b"Hi")).start();
}