//! Booleans.

use crate::object::Object;
use crate::object::macros::object;
use crate::object::name::read_word;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};

impl Skippable for bool {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        matches!(read_word(r)?, b"true" | b"false").then_some(())
    }
}

impl Readable<'_> for bool {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        match r.skip::<Self>(true)? {
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
    use crate::reader::ReaderExt;

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
        for word in ["trueabdf", "true1", "false0", "trust", "tru"] {
            assert_eq!(
                Reader::new(word.as_bytes()).read_without_context::<bool>(),
                None,
                "{word}"
            );
        }
        for (word, value) in [("true]", true), ("false/A", false), ("true%c", true)] {
            assert_eq!(
                Reader::new(word.as_bytes()).read_without_context::<bool>(),
                Some(value),
                "{word}"
            );
        }
    }
}
