//! Logging macros that optionally forward to the `log` crate.

macro_rules! ldebug {
    ($fmt:literal $(, $($arg:expr),* $(,)?)?) => {
        #[cfg(feature = "logging")]
        ::log::debug!($fmt $(, $($arg),*)?);
        #[cfg(not(feature = "logging"))]
        { $($(let _ = &$arg;)*)? }
    };
}

macro_rules! ltrace {
    ($fmt:literal $(, $($arg:expr),* $(,)?)?) => {
        #[cfg(feature = "logging")]
        ::log::trace!($fmt $(, $($arg),*)?);
        #[cfg(not(feature = "logging"))]
        { $($(let _ = &$arg;)*)? }
    };
}

macro_rules! lwarn {
    ($fmt:literal $(, $($arg:expr),* $(,)?)?) => {
        #[cfg(feature = "logging")]
        ::log::warn!($fmt $(, $($arg),*)?);
        #[cfg(not(feature = "logging"))]
        { $($(let _ = &$arg;)*)? }
    };
}
