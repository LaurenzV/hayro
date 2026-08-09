//! The null object.

use crate::object::Object;
use crate::object::macros::object;
use crate::reader::Reader;
use crate::reader::{ByteReader, Readable, ReaderContext, Skippable};
use core::fmt::{Display, Formatter};

/// The null object.
#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct Null;

impl Display for Null {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.write_str("null")
    }
}

object!(Null, Null);

impl Skippable for Null {
    fn skip(r: &mut Reader<'_, '_>, _: bool) -> Option<()> {
        r.forward_tag(b"null")
    }
}

impl Readable<'_> for Null {
    fn read(r: &mut Reader<'_, '_>, ctx: &ReaderContext<'_>) -> Option<Self> {
        Self::skip(r, ctx.in_content_stream())?;

        Some(Self)
    }
}

#[cfg(test)]
mod tests {
    use crate::object::Null;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn display() {
        assert_eq!(format!("{}", Null), "null");
    }

    #[test]
    fn null() {
        assert_eq!(
            Reader::from_slice("null".as_bytes())
                .read_without_context::<Null>()
                .unwrap(),
            Null
        );
    }

    #[test]
    fn null_trailing() {
        assert_eq!(
            Reader::from_slice("nullabs".as_bytes())
                .read_without_context::<Null>()
                .unwrap(),
            Null
        );
    }
}
