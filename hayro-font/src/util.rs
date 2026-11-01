use hayro_syntax::object::{Dict, Stream};
use siphasher::sip128::{Hasher128, SipHasher13};
use std::hash::Hash;

pub(crate) fn hash128<T: Hash + ?Sized>(value: &T) -> u128 {
    let mut state = SipHasher13::new();
    value.hash(&mut state);
    state.finish128().as_u128()
}

pub(crate) trait CacheKey {
    fn cache_key(&self) -> u128;
}

impl CacheKey for Dict<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.data())
    }
}

impl CacheKey for Stream<'_> {
    fn cache_key(&self) -> u128 {
        match self.dict().obj_id() {
            Some(obj_id) => hash128(&obj_id),
            None => hash128(&(self.dict().data(), self.raw_data().as_ref())),
        }
    }
}
