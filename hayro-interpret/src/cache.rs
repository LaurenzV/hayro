use crate::util::hash128;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Null, ObjRef, Object, Stream};
use hayro_syntax::page::Resources;
use kurbo::{Affine, Rect};
use rustc_hash::FxHashMap;
use std::any::Any;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex};

type CacheMap = FxHashMap<u128, Option<Box<dyn Any + Send + Sync>>>;
#[derive(Clone)]
pub(crate) struct Cache(Arc<Mutex<CacheMap>>);

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(FxHashMap::default())))
    }

    pub(crate) fn get_or_insert_with<T: Clone + Send + Sync + 'static>(
        &self,
        id: u128,
        f: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let mut locked = self.0.lock().unwrap();

        // We can't use `get_or_insert_with` here, because if the closure makes another access to the
        // cache, we end up with a deadlock.
        match locked.entry(id) {
            Entry::Occupied(o) => o
                .get()
                .as_ref()
                .and_then(|val| val.downcast_ref::<T>().cloned()),
            Entry::Vacant(_) => {
                drop(locked);
                let val = f();
                self.0.lock().unwrap().insert(
                    id,
                    val.clone()
                        .map(|val| Box::new(val) as Box<dyn Any + Send + Sync>),
                );

                val
            }
        }
    }
}

/// A trait for objects that can generate a unique cache key.
pub trait CacheKey {
    /// Returns the cache key for this object.
    fn cache_key(&self) -> u128;
}

impl<T: CacheKey, U: CacheKey> CacheKey for (T, U) {
    fn cache_key(&self) -> u128 {
        hash128(&(self.0.cache_key(), self.1.cache_key()))
    }
}

impl CacheKey for Dict<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.data())
    }
}

impl CacheKey for Stream<'_> {
    fn cache_key(&self) -> u128 {
        match self.dict().obj_id() {
            // A stream written directly inside an indirect object, which we
            // accept although it's invalid, has the identifier of that object,
            // so we tell it apart by where its dictionary is. The address is
            // stable: the bytes of an object live as long as its document.
            Some(obj_id) => hash128(&(obj_id, self.dict().data().as_ptr() as usize)),
            None => hash128(&(self.dict().data(), self.raw_data().as_ref())),
        }
    }
}

impl CacheKey for Resources<'_> {
    fn cache_key(&self) -> u128 {
        hash128(
            &[
                &self.ext_g_states,
                &self.fonts,
                &self.properties,
                &self.color_spaces,
                &self.x_objects,
                &self.patterns,
                &self.shadings,
            ]
            .map(Dict::cache_key),
        )
    }
}

impl CacheKey for Null {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for bool {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for hayro_syntax::object::Number {
    fn cache_key(&self) -> u128 {
        hash128(&self.as_f64().to_bits())
    }
}

impl CacheKey for hayro_syntax::object::String<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.as_ref())
    }
}

impl CacheKey for Name<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for Array<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.data())
    }
}

impl CacheKey for Object<'_> {
    fn cache_key(&self) -> u128 {
        match self {
            Object::Null(n) => n.cache_key(),
            Object::Boolean(b) => b.cache_key(),
            Object::Number(n) => n.cache_key(),
            Object::String(s) => s.cache_key(),
            Object::Name(n) => n.cache_key(),
            Object::Dict(d) => d.cache_key(),
            Object::Array(a) => a.cache_key(),
            Object::Stream(s) => s.cache_key(),
        }
    }
}

impl CacheKey for ObjRef {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl<T: CacheKey> CacheKey for MaybeRef<T> {
    fn cache_key(&self) -> u128 {
        match self {
            Self::Ref(r) => r.cache_key(),
            Self::NotRef(o) => o.cache_key(),
        }
    }
}

impl CacheKey for Affine {
    fn cache_key(&self) -> u128 {
        let c = self.as_coeffs();
        hash128(&[
            c[0].to_bits(),
            c[1].to_bits(),
            c[2].to_bits(),
            c[3].to_bits(),
            c[4].to_bits(),
            c[5].to_bits(),
        ])
    }
}

impl CacheKey for Rect {
    fn cache_key(&self) -> u128 {
        hash128(&[
            self.x0.to_bits(),
            self.y0.to_bits(),
            self.x1.to_bits(),
            self.y1.to_bits(),
        ])
    }
}

impl CacheKey for u128 {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

#[cfg(test)]
mod tests {
    use super::CacheKey;
    use hayro_syntax::Pdf;
    use hayro_syntax::object::{Dict, ObjectIdentifier, Stream};

    fn pdf(objects: &[&[u8]]) -> Pdf {
        let mut data = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![];

        for (i, body) in objects.iter().enumerate() {
            offsets.push(data.len());
            data.extend(format!("{} 0 obj\n", i + 1).bytes());
            data.extend(*body);
            data.extend(b"\nendobj\n");
        }

        let xref = data.len();
        data.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());

        for offset in offsets {
            data.extend(format!("{offset:010} 00000 n \n").bytes());
        }

        data.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );

        Pdf::new(data).unwrap()
    }

    #[test]
    fn direct_streams_in_one_object_have_distinct_keys() {
        let pdf = pdf(&[
            b"<< /Type /Catalog /Pages 2 0 R
/A << /Length 4 /Filter /ASCIIHexDecode >>
stream
0 0 
endstream
/B << /Length 4 /Filter /ASCIIHexDecode >>
stream
0 1 
endstream
>>",
            b"<< /Type /Pages /Kids [] /Count 0 >>",
        ]);
        let dict = pdf
            .xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(1, 0))
            .unwrap();
        let a = dict.get::<Stream<'_>>(b"A").unwrap();
        let b = dict.get::<Stream<'_>>(b"B").unwrap();

        assert_ne!(a.decoded().unwrap(), b.decoded().unwrap());
        assert_eq!(a.dict().obj_id(), b.dict().obj_id());
        assert_ne!(a.cache_key(), b.cache_key());
        assert_eq!(
            a.cache_key(),
            dict.get::<Stream<'_>>(b"A").unwrap().cache_key()
        );
    }

    #[test]
    fn indirect_stream_keeps_its_key() {
        let pdf = pdf(&[
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [] /Count 0 >>",
            b"<< /Length 2 >>\nstream\nab\nendstream",
        ]);
        let id = ObjectIdentifier::new(3, 0);
        let first = pdf.xref().get::<Stream<'_>>(id).unwrap();
        let second = pdf.xref().get::<Stream<'_>>(id).unwrap();

        assert_eq!(first.cache_key(), second.cache_key());
    }
}
