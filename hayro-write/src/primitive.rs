use crate::ExtractionContext;
use hayro_syntax::object;
use hayro_syntax::object::dict::keys::{
    AF, CRYPT, DECODE_PARMS, FILTER, LAST_MODIFIED, LENGTH, METADATA, OC, OPI, PIECE_INFO, PT_DATA,
    REF, STRUCT_PARENT, STRUCT_PARENTS,
};
use hayro_syntax::object::{MaybeRef, Null, Number, ObjectIdentifier, Stream};
use hayro_syntax::object::{Object, array, dict};
use pdf_writer::{Chunk, Dict, Obj, Ref};
use std::collections::HashSet;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::LazyLock;

static IGNORE_KEYS: LazyLock<HashSet<&'static [u8]>> = LazyLock::new(|| {
    let mut m = HashSet::new();

    m.insert(METADATA);
    m.insert(STRUCT_PARENT);
    m.insert(OC);
    m.insert(AF);
    m.insert(PT_DATA);
    m.insert(REF);
    m.insert(LAST_MODIFIED);
    m.insert(PIECE_INFO);
    m.insert(STRUCT_PARENTS);
    m.insert(OPI);

    m
});

pub(crate) trait WriteDirect {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>);
}

impl WriteDirect for object::ObjRef {
    fn write_direct(&self, obj: Obj<'_>, ctx: &mut ExtractionContext<'_>) {
        // Check whether the object is actually valid/exists. Otherwise, we need to write a null
        // object instead. If we don't, we might end up with dangling object references that
        // don't actually exist in the PDF chunk, which will lead to a panic in krilla when renumbering.
        let valid = *ctx.valid_ref_cache.entry(*self).or_insert_with(|| {
            let id = ObjectIdentifier::new(self.obj_number, self.gen_number);
            ctx.pdf.xref().get::<Object<'_>>(id).is_some()
        });

        if valid {
            ctx.to_visit_refs.push(*self);
            let mapped_ref = ctx.map_ref(*self);
            obj.primitive(mapped_ref);
        } else {
            obj.primitive(pdf_writer::Null);
        }
    }
}

impl WriteDirect for Number {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>) {
        let float_num = self.as_f64();

        if float_num.fract() == 0.0 {
            obj.primitive(float_num as i32);
        } else {
            obj.primitive(float_num as f32);
        }
    }
}

impl WriteDirect for bool {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>) {
        obj.primitive(self);
    }
}

impl WriteDirect for Null {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>) {
        obj.primitive(pdf_writer::Null);
    }
}

impl WriteDirect for object::String<'_> {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>) {
        obj.primitive(pdf_writer::Str(self.as_ref()));
    }
}

impl WriteDirect for object::Name<'_> {
    fn write_direct(&self, obj: Obj<'_>, _: &mut ExtractionContext<'_>) {
        obj.primitive(pdf_writer::Name(self.deref()));
    }
}

impl WriteDirect for object::Array<'_> {
    fn write_direct(&self, obj: Obj<'_>, ctx: &mut ExtractionContext<'_>) {
        let mut arr = obj.array();
        for item in self.raw_iter() {
            let obj = arr.push();
            item.write_direct(obj, ctx);
        }
    }
}

impl<T: WriteDirect> WriteDirect for MaybeRef<T> {
    fn write_direct(&self, obj: Obj<'_>, ctx: &mut ExtractionContext<'_>) {
        match self {
            Self::Ref(r) => r.write_direct(obj, ctx),
            Self::NotRef(o) => o.write_direct(obj, ctx),
        }
    }
}

fn write_dict(
    hayro_dict: &dict::Dict<'_>,
    pdf_dict: &mut Dict<'_>,
    ctx: &mut ExtractionContext<'_>,
    skip_keys: &[&[u8]],
) {
    for (name, val) in hayro_dict.entries() {
        if skip_keys.contains(&name.deref()) {
            continue;
        }

        if !IGNORE_KEYS.contains(name.deref()) {
            val.write_direct(pdf_dict.insert(pdf_writer::Name(name.deref())), ctx);
        }
    }
}

impl WriteDirect for object::Dict<'_> {
    fn write_direct(&self, obj: Obj<'_>, ctx: &mut ExtractionContext<'_>) {
        let mut dict = obj.dict();

        write_dict(self, &mut dict, ctx, &[]);
    }
}

impl WriteDirect for Object<'_> {
    fn write_direct(&self, obj: Obj<'_>, ctx: &mut ExtractionContext<'_>) {
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

pub(crate) trait WriteIndirect {
    fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext<'_>);
}

macro_rules! write_indirect {
    ($name:ty) => {
        impl WriteIndirect for $name {
            fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext<'_>) {
                self.write_direct(chunk.indirect(id), ctx);
            }
        }
    };
}

write_indirect!(Null);
write_indirect!(bool);
write_indirect!(Number);
write_indirect!(object::String<'_>);
write_indirect!(object::Name<'_>);
write_indirect!(dict::Dict<'_>);
write_indirect!(array::Array<'_>);

impl WriteIndirect for Stream<'_> {
    fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext<'_>) {
        let data = self.raw_data();
        let mut obj = chunk.stream(id, &data);
        let dict = self.dict();

        if !has_crypt_filter(dict) {
            write_dict(dict, obj.deref_mut(), ctx, &[LENGTH]);
            return;
        }

        // `raw_data` has already applied the /Crypt filters, and the copy isn't
        // encrypted, so they are left out with their parameters. An entry that
        // isn't an array only belongs to the /Crypt filter.
        write_dict(dict, obj.deref_mut(), ctx, &[LENGTH, FILTER, DECODE_PARMS]);
        let Some(filters) = dict.get::<array::Array<'_>>(FILTER) else {
            return;
        };
        let kept = filters
            .raw_iter()
            .map(|filter| !is_crypt(&filter))
            .collect::<Vec<_>>();

        if !kept.contains(&true) {
            return;
        }

        for key in [FILTER, DECODE_PARMS] {
            if let Some(array) = dict.get::<array::Array<'_>>(key) {
                let mut written = obj.insert(pdf_writer::Name(key)).array();

                for (item, _) in array.raw_iter().zip(&kept).filter(|(_, kept)| **kept) {
                    item.write_direct(written.push(), ctx);
                }
            }
        }
    }
}

fn is_crypt(filter: &MaybeRef<Object<'_>>) -> bool {
    matches!(filter, MaybeRef::NotRef(Object::Name(name)) if name.deref() == CRYPT)
}

fn has_crypt_filter(dict: &dict::Dict<'_>) -> bool {
    if let Some(name) = dict.get::<object::Name<'_>>(FILTER) {
        name.deref() == CRYPT
    } else {
        dict.get::<array::Array<'_>>(FILTER)
            .is_some_and(|filters| filters.raw_iter().any(|filter| is_crypt(&filter)))
    }
}

impl WriteIndirect for Object<'_> {
    fn write_indirect(&self, chunk: &mut Chunk, id: Ref, ctx: &mut ExtractionContext<'_>) {
        match self {
            Object::Null(n) => n.write_indirect(chunk, id, ctx),
            Object::Boolean(b) => b.write_indirect(chunk, id, ctx),
            Object::Number(n) => n.write_indirect(chunk, id, ctx),
            Object::String(s) => s.write_indirect(chunk, id, ctx),
            Object::Name(n) => n.write_indirect(chunk, id, ctx),
            Object::Dict(d) => d.write_indirect(chunk, id, ctx),
            Object::Array(a) => a.write_indirect(chunk, id, ctx),
            Object::Stream(s) => s.write_indirect(chunk, id, ctx),
        }
    }
}
