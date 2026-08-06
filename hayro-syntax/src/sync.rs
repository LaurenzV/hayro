#[cfg(feature = "std")]
pub(crate) use std::collections::HashMap;

#[cfg(not(feature = "std"))]
pub(crate) use alloc::collections::BTreeMap as HashMap;

#[cfg(feature = "std")]
pub(crate) use rustc_hash::FxHashMap;

#[cfg(not(feature = "std"))]
pub(crate) use alloc::collections::BTreeMap as FxHashMap;

// Keep in sync with the implementation in `page`.
#[cfg(feature = "std")]
pub(crate) use std::sync::Arc;

#[cfg(not(feature = "std"))]
pub(crate) use alloc::rc::Rc as Arc;

#[cfg(feature = "std")]
pub(crate) use std::sync::OnceLock;

#[cfg(not(feature = "std"))]
pub(crate) use core::cell::OnceCell as OnceLock;

#[cfg(feature = "std")]
pub(crate) use std::sync::Mutex;

#[cfg(not(feature = "std"))]
pub(crate) use core::cell::RefCell as Mutex;

#[cfg(feature = "std")]
pub(crate) use std::sync::RwLock;

#[cfg(not(feature = "std"))]
pub(crate) use core::cell::RefCell as RwLock;

#[cfg(feature = "std")]
pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

#[cfg(not(feature = "std"))]
pub(crate) type MutexGuard<'a, T> = core::cell::RefMut<'a, T>;

#[cfg(feature = "std")]
pub(crate) type RwLockReadGuard<'a, T> = std::sync::RwLockReadGuard<'a, T>;

#[cfg(not(feature = "std"))]
pub(crate) type RwLockReadGuard<'a, T> = core::cell::Ref<'a, T>;

#[cfg(feature = "std")]
pub(crate) type RwLockWriteGuard<'a, T> = std::sync::RwLockWriteGuard<'a, T>;

#[cfg(not(feature = "std"))]
pub(crate) type RwLockWriteGuard<'a, T> = core::cell::RefMut<'a, T>;

pub(crate) trait MutexExt<T> {
    fn get(&self) -> MutexGuard<'_, T>;
}

#[cfg(feature = "std")]
impl<T> MutexExt<T> for Mutex<T> {
    fn get(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap()
    }
}

#[cfg(not(feature = "std"))]
impl<T> MutexExt<T> for Mutex<T> {
    fn get(&self) -> MutexGuard<'_, T> {
        self.borrow_mut()
    }
}

pub(crate) trait RwLockExt<T> {
    fn get(&self) -> RwLockReadGuard<'_, T>;
    fn put(&self) -> RwLockWriteGuard<'_, T>;
}

#[cfg(feature = "std")]
impl<T> RwLockExt<T> for RwLock<T> {
    fn get(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap()
    }

    fn put(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap()
    }
}

#[cfg(not(feature = "std"))]
impl<T> RwLockExt<T> for RwLock<T> {
    fn get(&self) -> RwLockReadGuard<'_, T> {
        self.borrow()
    }

    fn put(&self) -> RwLockWriteGuard<'_, T> {
        self.borrow_mut()
    }
}
