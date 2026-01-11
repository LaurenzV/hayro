//! Logging macros that optionally forward to the `log` crate.

macro_rules! ldebug {
    ($($arg:tt)*) => {
        #[cfg(feature = "logging")]
        ::log::debug!($($arg)*)
    };
}

macro_rules! ltrace {
    ($($arg:tt)*) => {
        #[cfg(feature = "logging")]
        ::log::trace!($($arg)*)
    };
}

macro_rules! lwarn {
    ($($arg:tt)*) => {
        #[cfg(feature = "logging")]
        ::log::warn!($($arg)*)
    };
}

pub(crate) use {ldebug, ltrace, lwarn};
