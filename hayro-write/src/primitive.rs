use crate::ExtractionContext;
use hayro_syntax::object::Object;
use hayro_syntax::object::r#ref::MaybeRef;
use pdf_writer::Obj;
use std::ops::Deref;

pub(crate) trait WritePrimitive {
    fn write(&self, obj: Obj, _: &mut ExtractionContext);
}

impl WritePrimitive for hayro_syntax::object::r#ref::ObjRef {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        ctx.to_visit_refs.push(*self);
        let mapped_ref = ctx.map_ref(*self);
        obj.primitive(mapped_ref);
    }
}

impl WritePrimitive for hayro_syntax::object::number::Number {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        let float_num = self.as_f64();

        if float_num.fract() == 0.0 {
            obj.primitive(float_num as i32);
        } else {
            obj.primitive(float_num as f32);
        }
    }
}

impl WritePrimitive for bool {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(self);
    }
}

impl WritePrimitive for hayro_syntax::object::null::Null {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Null);
    }
}

impl WritePrimitive for hayro_syntax::object::string::String<'_> {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Str(self.get().as_ref()))
    }
}

impl WritePrimitive for hayro_syntax::object::name::Name<'_> {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        obj.primitive(pdf_writer::Name(self.deref()));
    }
}

impl WritePrimitive for hayro_syntax::object::array::Array<'_> {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        let mut arr = obj.array();
        for item in self.raw_iter() {
            let obj = arr.push();
            item.write(obj, ctx);
        }
    }
}

impl<T: WritePrimitive> WritePrimitive for MaybeRef<T> {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        match self {
            MaybeRef::Ref(r) => r.write(obj, ctx),
            MaybeRef::NotRef(o) => o.write(obj, ctx),
        }
    }
}

impl WritePrimitive for hayro_syntax::object::dict::Dict<'_> {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        let mut dict = obj.dict();

        for (name, val) in self.entries() {
            val.write(dict.insert(pdf_writer::Name(name.deref())), ctx);
        }
    }
}

impl WritePrimitive for Object<'_> {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        match self {
            Object::Null(n) => n.write(obj, ctx),
            Object::Boolean(b) => b.write(obj, ctx),
            Object::Number(n) => n.write(obj, ctx),
            Object::String(s) => s.write(obj, ctx),
            Object::Name(n) => n.write(obj, ctx),
            Object::Dict(d) => d.write(obj, ctx),
            Object::Array(a) => a.write(obj, ctx),
            Object::Stream(_) => {}
        }
    }
}
