use crate::util::hash128;
use hayro_syntax::object::{Dict, ObjectIdentifier, Stream};
use std::any::Any;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
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
