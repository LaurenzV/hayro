use crate::util::hash128;
use hayro_syntax::object::{Dict, ObjectIdentifier, Stream};
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type CacheMap = HashMap<ObjectIdentifier, Option<Box<dyn Any + Send + Sync>>>;
#[derive(Clone)]
pub(crate) struct Cache(Arc<Mutex<CacheMap>>);

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn get_or_insert_with<T: Clone + Send + Sync + 'static>(
        &self,
        id: ObjectIdentifier,
        f: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        self.0
            .lock()
            .unwrap()
            .entry(id)
            .or_insert_with(|| f().map(|val| Box::new(val) as Box<dyn Any + Send + Sync>))
            .as_ref()
            .and_then(|val| val.downcast_ref::<T>().cloned())
    }
}

/// A trait for objects that can generate a unique cache key.
pub trait CacheKey {
    /// Returns the cache key for this object.
    fn cache_key(&self) -> u128;
}

impl CacheKey for Dict<'_> {
    fn cache_key(&self) -> u128 {
        self.obj_id()
            .map(|o| hash128(&o))
            .unwrap_or(hash128(self.data()))
    }
}

impl CacheKey for Stream<'_> {
    fn cache_key(&self) -> u128 {
        self.dict().cache_key()
    }
}
