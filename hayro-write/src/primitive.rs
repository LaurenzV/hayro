use crate::ExtractionContext;
use hayro_syntax::object::null::Null;
use hayro_syntax::object::number::Number;
use hayro_syntax::object::r#ref::MaybeRef;
use hayro_syntax::object::stream::Stream;
use hayro_syntax::object::{Object, array, dict, name, string};
use pdf_writer::{Chunk, Dict, Obj, Ref};
use std::ops::Deref;
use std::ops::DerefMut;

pub(crate) trait WriteDirect {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext);
}

impl WriteDirect for hayro_syntax::object::r#ref::ObjRef {
    fn write_direct(&self, obj: Obj, ctx: &mut ExtractionContext) {
        ctx.to_visit_refs.push(*self);
        let mapped_ref = ctx.map_ref(*self);
        obj.primitive(mapped_ref);
    }
}

impl WriteDirect for hayro_syntax::object::number::Number {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext) {
        let float_num = self.as_f64();

        if float_num.fract() == 0.0 {
            obj.primitive(float_num as i32);
        } else {
            obj.primitive(float_num as f32);
        }
    }
}

impl WriteDirect for bool {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(self);
    }
}

impl WriteDirect for hayro_syntax::object::null::Null {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Null);
    }
}

impl WriteDirect for hayro_syntax::object::string::String<'_> {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Str(self.get().as_ref()))
    }
}

impl WriteDirect for hayro_syntax::object::name::Name<'_> {
    fn write_direct(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Name(self.deref()));
    }
}

impl WriteDirect for hayro_syntax::object::array::Array<'_> {
    fn write_direct(&self, obj: Obj, ctx: &mut ExtractionContext) {
        let mut arr = obj.array();
        for item in self.raw_iter() {
            let obj = arr.push();
            item.write_direct(obj, ctx);
        }
    }
}

impl<T: WriteDirect> WriteDirect for MaybeRef<T> {
    fn write_direct(&self, obj: Obj, ctx: &mut ExtractionContext) {
        match self {
            MaybeRef::Ref(r) => r.write_direct(obj, ctx),
            MaybeRef::NotRef(o) => o.write_direct(obj, ctx),
        }
    }
}

fn write_dict(hayro_dict: &dict::Dict, pdf_dict: &mut Dict, ctx: &mut ExtractionContext) {
    for (name, val) in hayro_dict.entries() {
        val.write_direct(pdf_dict.insert(pdf_writer::Name(name.deref())), ctx);
    }
}

impl WriteDirect for hayro_syntax::object::dict::Dict<'_> {
    fn write_direct(&self, obj: Obj, ctx: &mut ExtractionContext) {
        let mut dict = obj.dict();

        write_dict(self, &mut dict, ctx);
    }
}

impl WriteDirect for Object<'_> {
    fn write_direct(&self, obj: Obj, ctx: &mut ExtractionContext) {
        match self {
            Object::Null(n) => n.write_direct(obj, ctx),
            Object::Boolean(b) => b.write_direct(obj, ctx),
            Object::Number(n) => n.write_direct(obj, ctx),
            Object::String(s) => s.write_direct(obj, ctx),
            Object::Name(n) => n.write_direct(obj, ctx),
            Object::Dict(d) => d.write_direct(obj, ctx),
            Object::Array(a) => a.write_direct(obj, ctx),
            Object::Stream(_) => unreachable!(),
        }
    }
}

trait WriteIndirect {
    fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext);
}

macro_rules! write_indirect {
    ($name:ty) => {
        impl WriteIndirect for $name {
            fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext) {
                self.write_direct(chunk.indirect(id), ctx);
            }
        }
    };
}

write_indirect!(Null);
write_indirect!(bool);
write_indirect!(Number);
write_indirect!(string::String<'_>);
write_indirect!(name::Name<'_>);
write_indirect!(dict::Dict<'_>);
write_indirect!(array::Array<'_>);

impl WriteIndirect for Stream<'_> {
    fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext) {
        // TODO: Handle `Crypt` filter
        let mut obj = chunk.stream(id, self.raw_data());
        write_dict(self.dict(), obj.deref_mut(), ctx);
    }
}
