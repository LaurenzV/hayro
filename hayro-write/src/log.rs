//! Logging macros that optionally forward to the `log` crate.

macro_rules! warn {
    ($fmt:literal $(, $($arg:expr),* $(,)?)?) => {{
        #[cfg(feature = "logging")]
        {
            ::log::warn!($fmt $(, $($arg),*)?);
        }
        #[cfg(not(feature = "logging"))]
        {
            $($(let _ = &$arg;)*)?
        }
    }};
}
