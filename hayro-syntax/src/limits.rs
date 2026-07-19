//! Resource limits for parsing and decoding.

/// An upper bound on a resource, or no bound at all.
///
/// Prefer this over `Option<T>` for a limit: the "no bound" case is the
/// explicit [`Limit::Unlimited`], never an absent/`None` value that reads like
/// "unset" but actually removes the guard. Only reach for [`Limit::Unlimited`]
/// with trusted input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit<T> {
    /// Reject anything exceeding this value.
    AtMost(T),
    /// Impose no limit.
    Unlimited,
}

impl<T: Copy + Ord> Limit<T> {
    /// Whether `value` is within this limit (always true when
    /// [`Unlimited`](Limit::Unlimited)).
    pub fn permits(self, value: T) -> bool {
        match self {
            Self::AtMost(max) => value <= max,
            Self::Unlimited => true,
        }
    }

    /// The bound as an `Option`: `Some(max)` when bounded, `None` when
    /// [`Unlimited`](Limit::Unlimited).
    pub fn bound(self) -> Option<T> {
        match self {
            Self::AtMost(max) => Some(max),
            Self::Unlimited => None,
        }
    }
}

/// Resource limits applied while decoding the streams of a PDF file.
///
/// The limits are set when loading a document (see
/// [`Pdf::new_with_options`](crate::Pdf::new_with_options)) and govern all
/// processing of that document, including by consumers such as
/// `hayro-interpret`. The defaults are generous but bounded, so a crafted
/// file cannot force unbounded allocations while legitimate documents remain
/// unaffected. Use [`Limits::no_limits`] to disable every check.
///
/// Enforcing a limit is a routine, expected outcome — a deliberately tight
/// limit will skip large streams and images even on non-malicious files — so
/// it is reported at `debug` level, not `warn`. An over-limit stream fails to
/// decode; an over-limit image is skipped.
///
/// Note that streams parsed outside of a loaded document (for example while
/// the cross-reference table itself is being built, or inline images inside
/// content streams) are checked against [`Limits::DEFAULT`] rather than the
/// configured values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The maximum decompressed size of a single stream, in bytes.
    ///
    /// This bounds decompression bombs: chained `/Filter` arrays amplify
    /// roughly 1000x per `FlateDecode`/`LZWDecode` layer (64x for
    /// `RunLengthDecode`), so a tiny stream can otherwise expand to
    /// gigabytes. A stream exceeding the limit is treated as a decode
    /// failure.
    pub max_decoded_stream_size: Limit<usize>,
    /// The maximum width or height of an embedded image, in pixels.
    ///
    /// Enforced before any decode work; an image exceeding it is skipped.
    pub max_image_dimension: Limit<u32>,
    /// The maximum total number of pixels (width × height) of an embedded
    /// image.
    ///
    /// This bounds decode-time allocations, which are proportional to the
    /// claimed pixel count rather than the stream size. An image exceeding it
    /// is skipped.
    pub max_image_pixels: Limit<u64>,
}

impl Limits {
    /// The default limits: 512 MiB per decoded stream, 65535 pixels per
    /// image side and 500 million pixels per image.
    pub const DEFAULT: Self = Self {
        max_decoded_stream_size: Limit::AtMost(512 * 1024 * 1024),
        max_image_dimension: Limit::AtMost(65535),
        max_image_pixels: Limit::AtMost(500_000_000),
    };

    /// No limits at all: every check is disabled, restoring the unbounded
    /// behavior of earlier versions. Only use this with trusted input.
    pub const fn no_limits() -> Self {
        Self {
            max_decoded_stream_size: Limit::Unlimited,
            max_image_dimension: Limit::Unlimited,
            max_image_pixels: Limit::Unlimited,
        }
    }

    /// Whether an image of the given dimensions is within the configured
    /// limits.
    ///
    /// Returns `false` if either side exceeds
    /// [`max_image_dimension`](Self::max_image_dimension) or the total pixel
    /// count exceeds [`max_image_pixels`](Self::max_image_pixels). Decoders
    /// call this with the *actual* dimensions they are about to allocate for —
    /// which, for tiled or codestream-driven formats (JPEG 2000, CCITT,
    /// JBIG2), come from the codec's own headers rather than the image
    /// dictionary — so the limit bounds the allocation regardless of what the
    /// dictionary claimed.
    pub fn permits_image(&self, width: u32, height: u32) -> bool {
        self.max_image_dimension.permits(width)
            && self.max_image_dimension.permits(height)
            && self
                .max_image_pixels
                .permits(u64::from(width) * u64::from(height))
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::{Limit, Limits};

    #[test]
    fn limit_permits_and_bound() {
        assert!(Limit::AtMost(10u32).permits(10));
        assert!(!Limit::AtMost(10u32).permits(11));
        assert!(Limit::<u32>::Unlimited.permits(u32::MAX));

        assert_eq!(Limit::AtMost(10u32).bound(), Some(10));
        assert_eq!(Limit::<u32>::Unlimited.bound(), None);
    }

    #[test]
    fn permits_image_bounds_both_axes() {
        let limits = Limits {
            max_image_dimension: Limit::AtMost(100),
            max_image_pixels: Limit::AtMost(1000),
            ..Limits::default()
        };
        assert!(limits.permits_image(20, 20)); // 400 px, within both
        assert!(!limits.permits_image(101, 1)); // side exceeds dimension limit
        assert!(!limits.permits_image(40, 40)); // 1600 px exceeds pixel limit

        // Unlimited disables the check entirely.
        assert!(Limits::no_limits().permits_image(u32::MAX, u32::MAX));
    }
}
