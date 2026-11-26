use crate::object::ObjectIdentifier;
use crate::object::ObjectLike;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_regular_character;
use core::fmt::{Debug, Display, Formatter};

/// A reference to an object.
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub struct ObjRef {
    /// The object number.
    pub obj_number: u32,
    /// The generation number.
    pub gen_number: i32,
}

impl ObjRef {
    /// Create a new object reference.
    pub fn new(obj_number: u32, gen_number: i32) -> Self {
        Self {
            obj_number,
            gen_number,
        }
    }
}

impl Display for ObjRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} {} R", self.obj_number, self.gen_number)
    }
}

impl From<ObjRef> for ObjectIdentifier {
    fn from(value: ObjRef) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl From<ObjectIdentifier> for ObjRef {
    fn from(value: ObjectIdentifier) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl Skippable for ObjRef {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        Self::read(r, ReaderContext::dummy_ref()).map(|_| ())
    }
}

impl Readable<'_> for ObjRef {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let obj_number = read_ref_integer(r)?.try_into().ok()?;
        let gen_number = read_ref_integer(r)?.try_into().ok()?;
        r.forward_tag(b"R")?;
        if r.peek_byte().is_some_and(is_regular_character) {
            return None;
        }
        Some(Self::new(obj_number, gen_number))
    }
}

/// Either an object or a reference to one.
#[derive(PartialEq, Eq)]
pub enum MaybeRef<T> {
    /// A reference to an object.
    Ref(ObjRef),
    /// An object.
    NotRef(T),
}

impl<T> MaybeRef<T> {
    /// If the object is an object reference, return it.
    pub fn as_obj_ref(&self) -> Option<ObjRef> {
        match self {
            Self::Ref(r) => Some(*r),
            Self::NotRef(_) => None,
        }
    }
}

#[allow(private_bounds)]
impl<'a, T> MaybeRef<T>
where
    T: ObjectLike<'a>,
{
    pub(crate) fn resolve(self, ctx: &ReaderContext<'a>) -> Option<T> {
        match self {
            Self::Ref(r) => ctx.xref().get_with::<T>(r.into(), ctx),
            Self::NotRef(t) => Some(t),
        }
    }
}

impl<T> TryFrom<MaybeRef<T>> for ObjRef {
    type Error = ();

    fn try_from(value: MaybeRef<T>) -> Result<Self, Self::Error> {
        match value {
            MaybeRef::Ref(r) => Ok(r),
            MaybeRef::NotRef(_) => Err(()),
        }
    }
}

impl<T> Debug for MaybeRef<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ref(r) => write!(f, "{r:?}"),
            Self::NotRef(nr) => write!(f, "{nr:?}"),
        }
    }
}

impl<T: Display> Display for MaybeRef<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ref(value) => Display::fmt(value, f),
            Self::NotRef(value) => Display::fmt(value, f),
        }
    }
}

impl<T> Skippable for MaybeRef<T>
where
    T: Skippable,
{
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        Self::skip_with_depth(r, is_content_stream, 0)
    }

    fn skip_with_depth(r: &mut Reader<'_>, is_content_stream: bool, depth: usize) -> Option<()> {
        if !is_content_stream && r.skip::<ObjRef>(false).is_some() {
            return Some(());
        }
        r.skip_with_depth::<T>(is_content_stream, depth).map(|_| ())
    }
}

impl<'a, T> Readable<'a> for MaybeRef<T>
where
    T: Readable<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        if !ctx.in_content_stream()
            && let Some(obj) = r.read::<ObjRef>(ctx)
        {
            Some(Self::Ref(obj))
        } else {
            Some(Self::NotRef(r.read::<T>(ctx)?))
        }
    }
}

fn read_ref_integer(r: &mut Reader<'_>) -> Option<i64> {
    let negative = r.eat(|b| matches!(b, b'+' | b'-')) == Some(b'-');
    let start = r.offset();
    let mut value = 0_i64;
    while let Some(digit) = r.eat(|b| b.is_ascii_digit()) {
        value = value
            .checked_mul(10)?
            .checked_add(i64::from(digit - b'0'))?;
    }
    let has_digits = r.offset() != start;
    if r.forward_tag(b".").is_some() {
        if !has_digits {
            r.forward_tag(b"0")?;
        }
        r.forward_while(|b| b == b'0');
    } else if !has_digits {
        return None;
    }
    let end = r.offset();
    r.skip_white_spaces_and_comments();
    (r.offset() != end).then_some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use crate::object::{MaybeRef, Number, ObjRef};
    use crate::reader::ReaderExt;
    use crate::reader::{Reader, ReaderContext};

    #[test]
    fn display() {
        assert_eq!(format!("{}", ObjRef::new(34, 1)), "34 1 R");
        assert_eq!(format!("{}", MaybeRef::NotRef(Number::from_i32(12))), "12");
    }

    #[test]
    fn ref_1() {
        assert_eq!(
            Reader::new("34 1 R".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(34, 1)
        );
    }

    #[test]
    fn ref_trailing() {
        assert_eq!(
            Reader::new("256 0 R (hi)".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(256, 0)
        );
    }

    #[test]
    fn ref_invalid_1() {
        assert!(
            Reader::new("256 R".as_bytes())
                .read_without_context::<ObjRef>()
                .is_none()
        );
    }

    #[test]
    fn ref_invalid_2() {
        assert!(
            Reader::new("256 257".as_bytes())
                .read_without_context::<ObjRef>()
                .is_none()
        );
    }

    #[test]
    fn reference_number_boundaries() {
        let ctx = ReaderContext::dummy_ref();
        for (input, expected) in [
            ("8 .R", None),
            ("8 0R", None),
            ("8 . R", None),
            ("8 0.9 R", None),
            ("8.9 0 R", None),
            ("8.00000000000000001 0 R", None),
            ("8 0.0 Rfalsehood", None),
            ("3% note\n0 R", Some(ObjRef::new(3, 0))),
            ("3 0.0 R", Some(ObjRef::new(3, 0))),
            ("8 4294967296 R", None),
        ] {
            let mut r = Reader::new(input.as_bytes());
            assert_eq!(
                r.skip::<ObjRef>(ctx.in_content_stream()),
                expected.map(|_| input.as_bytes())
            );
            r.jump(0);
            assert_eq!(r.read::<ObjRef>(ctx), expected);
            assert_eq!(r.offset(), expected.map_or(0, |_| input.len()));
        }
    }

    #[test]
    fn ref_above_i32_max() {
        assert_eq!(
            Reader::new("2147483648 0 R".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(2_147_483_648, 0)
        );
        assert_eq!(
            Reader::new("4294967295 0 R".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(u32::MAX, 0)
        );
    }

    #[test]
    fn ref_invalid_object_number() {
        for input in ["4294967296 0 R", "-1 0 R"] {
            assert!(
                Reader::new(input.as_bytes())
                    .read_without_context::<ObjRef>()
                    .is_none()
            );
        }
    }
}
