use std::ops::Deref;
use pdf_writer::Obj;
use hayro_syntax::object::Object;
use crate::ExtractionContext;

pub(crate) trait WritePrimitive {
    fn write(&self, obj: Obj, _: &mut ExtractionContext);
}

impl WritePrimitive for hayro_syntax::object::r#ref::ObjRef {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        let mapped_ref = ctx.map_ref(self.clone());
        obj.primitive(mapped_ref);
    }
}

impl WritePrimitive for hayro_syntax::object::number::Number {
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        let float_num = self.as_f64();

        if float_num.fract() == 0.0 {
            obj.primitive(float_num as i32);
        }   else {
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
    fn write(&self, obj: Obj, _: &mut ExtractionContext) {
        let mut iter = self.flex_iter();
        for item in iter.next()
    }
}

impl WritePrimitive for hayro_syntax::object::Object<'_> {
    fn write(&self, obj: Obj, ctx: &mut ExtractionContext) {
        match self {
            Object::Null(n) => n.write(obj, ctx),
            Object::Boolean(b) => b.write(obj, ctx),
            Object::Number(n) => n.write(obj, ctx),
            Object::String(s) => s.write(obj, ctx),
            Object::Name(n) => n.write(obj, ctx),
            Object::Dict(_) => {}
            Object::Array(_) => {}
            Object::Stream(_) => {}
        }
    }
}