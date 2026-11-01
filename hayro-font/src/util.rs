use hayro_syntax::object::Dict;
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
