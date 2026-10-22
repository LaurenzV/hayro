use crate::util::hash128;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Null, ObjRef, Object, Stream};
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
        // Streams with byte-identical dictionaries can still hold different data. Within a
        // document, the object identifier tells them apart at a cost that doesn't grow with the
        // data; the dictionary keeps apart what it did before. An inline image has no object, so
        // its data takes the identifier's place.
        match self.dict().obj_id() {
            Some(obj_id) => hash128(&(obj_id, self.dict().data())),
            None => hash128(&(self.dict().data(), self.raw_data().as_ref())),
        }
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
    use super::*;
    use crate::color::{ColorSpace, ToRgb};
    use hayro_syntax::Pdf;
    use hayro_syntax::object::ObjectIdentifier;

    fn icc_pdf(gammas: &[f32]) -> Pdf {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        ];
        for (index, &gamma) in gammas.iter().enumerate() {
            let profile = moxcms::ColorProfile::new_gray_with_gamma(gamma)
                .encode()
                .unwrap();
            let mut stream = format!("<< /N 1 /Length {} >>\nstream\n", profile.len()).into_bytes();
            stream.extend(profile);
            stream.extend(b"\nendstream");
            objects.push(stream);
            objects.push(format!("[/ICCBased {} 0 R]", 3 + index * 2).into_bytes());
        }
        let mut data = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            data.extend(format!("{} 0 obj\n", index + 1).bytes());
            data.extend(object);
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

    fn stream(pdf: &Pdf, index: usize) -> Stream<'_> {
        pdf.xref()
            .get(ObjectIdentifier::new(3 + index as i32 * 2, 0))
            .unwrap()
    }

    fn gray_pixel(pdf: &Pdf, index: usize, cache: &Cache) -> [u8; 3] {
        let object = pdf
            .xref()
            .get(ObjectIdentifier::new(4 + index as i32 * 2, 0))
            .unwrap();
        let color = ColorSpace::new(object, cache).unwrap();
        let mut pixel = [0; 3];
        color.convert(&[128], &mut pixel).unwrap();
        pixel
    }

    #[test]
    fn equal_stream_dictionaries_do_not_share_icc_colors() {
        let pdf = icc_pdf(&[1.0, 2.2]);
        assert_eq!(stream(&pdf, 0).dict().data(), stream(&pdf, 1).dict().data());
        let expected = [
            gray_pixel(&pdf, 0, &Cache::new()),
            gray_pixel(&pdf, 1, &Cache::new()),
        ];
        assert_ne!(expected[0], expected[1]);
        for order in [[0, 1], [1, 0]] {
            let cache = Cache::new();
            for index in order {
                assert_eq!(gray_pixel(&pdf, index, &cache), expected[index]);
            }
        }
    }

    #[test]
    fn resolving_the_same_stream_reuses_its_cache_entry() {
        let pdf = icc_pdf(&[1.0]);
        let cache = Cache::new();
        let key = stream(&pdf, 0).cache_key();
        assert_eq!(cache.get_or_insert_with(key, || Some(42_u8)), Some(42));
        let cloned = stream(&pdf, 0).clone();
        assert_eq!(
            cache.get_or_insert_with(cloned.cache_key(), || panic!("stream was not cached")),
            Some(42_u8)
        );
    }

    /// A stream's key is its object's, read without its bytes: two objects with equal
    /// dictionaries and bytes have two keys.
    #[test]
    fn a_stream_is_keyed_by_its_object_not_its_bytes() {
        let pdf = icc_pdf(&[1.0, 1.0]);
        let (a, b) = (stream(&pdf, 0), stream(&pdf, 1));
        assert_eq!(
            (a.dict().data(), a.raw_data()),
            (b.dict().data(), b.raw_data())
        );
        assert_ne!(a.cache_key(), b.cache_key());
    }
}
