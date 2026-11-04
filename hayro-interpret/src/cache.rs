use crate::util::hash128;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Null, ObjRef, Object, Stream};
use hayro_syntax::page::Resources;
use kurbo::{Affine, Rect};
use rustc_hash::FxHashMap;
use std::any::Any;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type CacheMap = FxHashMap<u128, Option<Box<dyn Any + Send + Sync>>>;

/// VeriPDF: entries also count towards a byte budget, so caching decoded
/// images (which can be tens of megabytes) cannot grow without bound on a
/// hostile file. Budget overruns drop every entry rather than the newest
/// one: all values are cheap to re-derive.
const CACHE_BUDGET_BYTES: u64 = 192 << 20;

#[derive(Default)]
struct CacheState {
    map: CacheMap,
    /// VeriPDF: insertion order and sizes for LRU eviction — a page full
    /// of large images overflows any fixed budget, and clearing
    /// everything (this patch's first version) threw away the fonts along
    /// with the images on every tile. Evicting oldest-first keeps the hot
    /// end warm.
    order: std::collections::VecDeque<u128>,
    sizes: FxHashMap<u128, u64>,
    bytes: u64,
    hits: AtomicU64,
    misses: AtomicU64,
}

/// VeriPDF: `pub` so a consumer can construct one cache per document and
/// hand a clone to every per-page/per-tile `RenderCache`, keeping decoded
/// objects alive across renders.
#[derive(Clone)]
pub struct Cache(Arc<Mutex<CacheState>>);

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// VeriPDF: `pub` (was `pub(crate)`), see the struct docs.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(CacheState::default())))
    }

    /// VeriPDF: (hits, misses) across all budgeted accesses, for tests.
    pub fn budgeted_counters(&self) -> (u64, u64) {
        let state = self.0.lock().unwrap();
        (
            state.hits.load(Ordering::Relaxed),
            state.misses.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn get_or_insert_with<T: Clone + Send + Sync + 'static>(
        &self,
        id: u128,
        f: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let mut locked = self.0.lock().unwrap();

        // We can't use `get_or_insert_with` here, because if the closure makes another access to the
        // cache, we end up with a deadlock.
        match locked.map.entry(id) {
            Entry::Occupied(o) => o
                .get()
                .as_ref()
                .and_then(|val| val.downcast_ref::<T>().cloned()),
            Entry::Vacant(_) => {
                drop(locked);
                let val = f();
                self.0.lock().unwrap().map.insert(
                    id,
                    val.clone()
                        .map(|val| Box::new(val) as Box<dyn Any + Send + Sync>),
                );

                val
            }
        }
    }

    /// VeriPDF: like [`Cache::get_or_insert_with`], but the value reports
    /// its own size in bytes and the cache is kept under
    /// [`CACHE_BUDGET_BYTES`]. The closure returns the value and its size
    /// together, so an over-budget insert can shed older entries.
    pub fn get_or_insert_with_budgeted<T: Clone + Send + Sync + 'static>(
        &self,
        id: u128,
        f: impl FnOnce() -> Option<(T, u64)>,
    ) -> Option<T> {
        let existing = {
            let locked = self.0.lock().unwrap();
            match locked.map.get(&id) {
                Some(cached) => {
                    let value = cached
                        .as_ref()
                        .and_then(|val| val.downcast_ref::<T>().cloned());
                    Some(value)
                }
                None => None,
            }
        };
        if let Some(hit) = existing {
            self.0.lock().unwrap().hits.fetch_add(1, Ordering::Relaxed);
            return hit;
        }

        let (val, bytes) = f()?;
        let mut locked = self.0.lock().unwrap();
        locked.misses.fetch_add(1, Ordering::Relaxed);
        // A value too big to ever fit comfortably is not cached at all:
        // inserting it would evict every other entry on every call (a
        // page-covering image on a huge page), which is strictly worse
        // than rendering it uncached.
        if bytes > CACHE_BUDGET_BYTES / 2 {
            return Some(val);
        }
        if locked.bytes + bytes > CACHE_BUDGET_BYTES {
            // VeriPDF: evict oldest entries until the newcomer fits;
            // everything evicted is re-derivable. A single entry larger
            // than the whole budget was already refused above.
            while locked.bytes + bytes > CACHE_BUDGET_BYTES
                && let Some(oldest) = locked.order.pop_front()
            {
                locked.bytes =
                    locked.bytes.saturating_sub(locked.sizes.get(&oldest).copied().unwrap_or(0));
                locked.sizes.remove(&oldest);
                locked.map.remove(&oldest);
            }
        }
        locked.bytes += bytes;
        locked.sizes.insert(id, bytes);
        locked.order.push_back(id);
        locked.map.insert(
            id,
            Some(Box::new(val.clone()) as Box<dyn Any + Send + Sync>),
        );
        Some(val)
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
            Some(obj_id) => hash128(&obj_id),
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

// VeriPDF: key material for the decode cache — a target dimension pairs
// with the stream identity, so the same image at different raster sizes is
// a different entry.
impl CacheKey for (u32, u32) {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl<T: CacheKey> CacheKey for Option<T> {
    fn cache_key(&self) -> u128 {
        match self {
            Some(t) => hash128(&t.cache_key()),
            None => hash128(&0u128),
        }
    }
}

// VeriPDF: composite key material.
impl CacheKey for [u128; 4] {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}
