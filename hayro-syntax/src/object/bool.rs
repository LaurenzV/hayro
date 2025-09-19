//! Booleans.

use crate::object::Object;
use crate::object::macros::object;
use crate::reader::{Readable, Reader, ReaderContext, Skippable};

impl Skippable for bool {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        match r.peek_byte()? {
            b't' => r.forward_tag(b"true"),
            b'f' => r.forward_tag(b"false"),
            _ => None,
        }
    }
}

impl Readable<'_> for bool {
    fn read(r: &mut Reader<'_>, _: &ReaderContext) -> Option<Self> {
        match r.skip_in_content_stream::<bool>()? {
            b"true" => Some(true),
            b"false" => Some(false),
            _ => None,
        }
    }
}

object!(bool, Boolean);

#[cfg(test)]
mod tests {
    use crate::reader::Reader;

    #[test]
    fn bool_true() {
        assert!(
            Reader::new("true".as_bytes())
                .read_without_context::<bool>()
                .unwrap()
        );
    }

    #[test]
    fn bool_false() {
        assert!(
            !Reader::new("false".as_bytes())
                .read_without_context::<bool>()
                .unwrap()
        );
    }

    #[test]
    fn bool_trailing() {
        assert!(
            Reader::new("trueabdf".as_bytes())
                .read_without_context::<bool>()
                .unwrap()
        );
    }
}
