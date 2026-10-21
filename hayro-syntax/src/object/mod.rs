//! Parsing and reading from PDF objects.

pub use crate::object::array::Array;
pub use crate::object::date::DateTime;
pub use crate::object::dict::Dict;
pub use crate::object::name::Name;
use crate::object::name::skip_name_like;
pub use crate::object::null::Null;
pub use crate::object::number::Number;
pub use crate::object::rect::Rect;
pub use crate::object::r#ref::{MaybeRef, ObjRef};
pub use crate::object::stream::Stream;
pub use crate::object::string::String;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use core::fmt::{Debug, Display, Formatter};

const MAX_OBJECT_NESTING_DEPTH: usize = 64;

mod bool;
mod date;
mod null;
mod number;
mod rect;
mod r#ref;
mod string;
mod tuple;

pub(crate) mod indirect;
pub(crate) mod name;

pub mod array;
pub mod dict;
pub mod stream;

/// A trait for PDF objects.
pub(crate) trait ObjectLike<'a>: TryFrom<Object<'a>> + Readable<'a> + Debug + Clone {}

/// A trait for PDF objects that can be borrowed directly from an [`Object`].
pub(crate) trait ObjectRefLike<'a>: Sized {
    fn cast_ref<'b>(obj: &'b Object<'a>) -> Option<&'b Self>;
}

/// A primitive PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object<'a> {
    /// A null object.
    Null(Null),
    /// A boolean object.
    Boolean(bool),
    /// A number object.
    Number(Number),
    /// A string object.
    String(String<'a>),
    /// A name object.
    Name(Name<'a>),
    /// A dict object.
    Dict(Dict<'a>),
    /// An array object.
    Array(Array<'a>),
    /// A stream object.
    // Can only be an indirect object in theory and thus comes with some caveats,
    // but we just treat it the same.
    Stream(Stream<'a>),
}

impl Display for Object<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Null(value) => Display::fmt(value, f),
            Self::Boolean(value) => Display::fmt(value, f),
            Self::Number(value) => Display::fmt(value, f),
            Self::String(value) => Display::fmt(value, f),
            Self::Name(value) => Display::fmt(value, f),
            Self::Dict(value) => Display::fmt(value, f),
            Self::Array(value) => Display::fmt(value, f),
            Self::Stream(value) => Display::fmt(value, f),
        }
    }
}

impl<'a> Object<'a> {
    /// Try casting the object to a specific subtype.
    pub(crate) fn cast<T>(self) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        self.try_into().ok()
    }

    /// Try casting the object to a reference of the specific type.
    pub(crate) fn cast_ref<T>(&self) -> Option<&T>
    where
        T: ObjectRefLike<'a>,
    {
        T::cast_ref(self)
    }

    /// Try casting the object to a dict.
    #[inline(always)]
    pub fn into_dict(self) -> Option<Dict<'a>> {
        self.cast()
    }

    /// Try casting the object to a name.
    #[inline(always)]
    pub fn into_name(self) -> Option<Name<'a>> {
        self.cast()
    }

    /// Try casting the object to the null object.
    #[inline(always)]
    pub fn into_null(self) -> Option<Null> {
        self.cast()
    }

    /// Try casting the object to a bool.
    #[inline(always)]
    pub fn into_bool(self) -> Option<bool> {
        self.cast()
    }

    /// Try casting the object to a string.
    #[inline(always)]
    pub fn into_string(self) -> Option<String<'a>> {
        self.cast()
    }

    /// Try casting the object to a stream.
    #[inline(always)]
    pub fn into_stream(self) -> Option<Stream<'a>> {
        self.cast()
    }

    /// Try casting the object to an array.
    #[inline(always)]
    pub fn into_array(self) -> Option<Array<'a>> {
        self.cast()
    }

    /// Try casting the object to a u8.
    #[inline(always)]
    pub fn into_u8(self) -> Option<u8> {
        self.cast()
    }

    /// Try casting the object to a u16.
    #[inline(always)]
    pub fn into_u16(self) -> Option<u16> {
        self.cast()
    }

    /// Try casting the object to a f32.
    #[inline(always)]
    pub fn into_f32(self) -> Option<f32> {
        self.cast()
    }

    /// Try casting the object to a i32.
    #[inline(always)]
    pub fn into_i32(self) -> Option<i32> {
        self.cast()
    }

    /// Try casting the object to a number.
    #[inline(always)]
    pub fn into_number(self) -> Option<Number> {
        self.cast()
    }
}

impl<'a> ObjectLike<'a> for Object<'a> {}
impl<'a> ObjectRefLike<'a> for Object<'a> {
    fn cast_ref(obj: &Self) -> Option<&Self> {
        Some(obj)
    }
}

impl Skippable for Object<'_> {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        Self::skip_with_depth(r, is_content_stream, 0)
    }

    fn skip_with_depth(r: &mut Reader<'_>, is_content_stream: bool, depth: usize) -> Option<()> {
        match r.peek_byte()? {
            b'n' => Null::skip(r, is_content_stream),
            b't' | b'f' => bool::skip(r, is_content_stream),
            b'/' => Name::skip(r, is_content_stream),
            b'<' => match r.peek_bytes(2)? {
                // A stream can never appear in a dict/array, so it should never be skipped.
                b"<<" => Dict::skip_with_depth(r, is_content_stream, depth),
                _ => String::skip(r, is_content_stream),
            },
            b'(' => String::skip(r, is_content_stream),
            b'.' | b'+' | b'-' | b'0'..=b'9' => match r.skip::<Number>(is_content_stream) {
                Some(_) => Some(()),
                // See the comment in `read`.
                None if !is_content_stream => skip_name_like(r, false),
                None => None,
            },
            b'[' => Array::skip_with_depth(r, is_content_stream, depth),
            // See test case operator-in-TJ-array-0: Be lenient and skip content operators in
            // array
            _ => skip_name_like(r, false),
        }
    }
}

impl<'a> Readable<'a> for Object<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let object = match r.peek_byte()? {
            b'n' => Self::Null(Null::read(r, ctx)?),
            b't' | b'f' => Self::Boolean(bool::read(r, ctx)?),
            b'/' => Self::Name(Name::read(r, ctx)?),
            b'<' => match r.peek_bytes(2)? {
                b"<<" => {
                    let mut cloned = r.clone();
                    let dict = Dict::read(&mut cloned, ctx)?;
                    cloned.skip_white_spaces_and_comments();

                    if cloned.forward_tag(b"stream").is_some() {
                        Object::Stream(Stream::read(r, ctx)?)
                    } else {
                        r.jump(cloned.offset());

                        Object::Dict(dict)
                    }
                }
                _ => Self::String(String::read(r, ctx)?),
            },
            b'(' => Self::String(String::read(r, ctx)?),
            b'.' | b'+' | b'-' | b'0'..=b'9' => match Number::read(r, ctx) {
                Some(number) => Self::Number(number),
                // Like MuPDF, read a token that starts like a number but isn't one, such as
                // `1e3` or `.foo`, as null, so that it doesn't make the object around it
                // unreadable. Content streams keep their own handling (see issue 994).
                None if !ctx.in_content_stream() => {
                    skip_name_like(r, false)?;
                    Self::Null(Null)
                }
                None => return None,
            },
            b'[' => Self::Array(Array::read(r, ctx)?),
            // See the comment in `skip`.
            _ => {
                skip_name_like(r, false)?;
                Self::Null(Null)
            }
        };

        Some(object)
    }
}

/// A trait for objects that can be parsed from a simple byte stream.
pub trait FromBytes<'a>: Sized {
    /// Try to read the object from the given bytes.
    fn from_bytes(b: &'a [u8]) -> Option<Self>;
}

impl<'a, T: Readable<'a>> FromBytes<'a> for T {
    fn from_bytes(b: &'a [u8]) -> Option<Self> {
        Self::from_bytes_impl(b)
    }
}

/// An identifier for a PDF object.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ObjectIdentifier {
    /// The object number.
    pub obj_number: i32,
    /// The generation number.
    pub gen_number: i32,
}

impl ObjectIdentifier {
    /// Create a new `ObjectIdentifier`.
    pub fn new(obj_number: i32, gen_number: i32) -> Self {
        Self {
            obj_number,
            gen_number,
        }
    }
}

impl Readable<'_> for ObjectIdentifier {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let obj_number = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        let gen_number = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"obj")?;

        Some(Self {
            obj_number,
            gen_number,
        })
    }
}

impl Skippable for ObjectIdentifier {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"obj")?;

        Some(())
    }
}

/// A convenience function that extracts a dict and a stream from an object.
/// If the object is just a dictionary, it will return `None` for the stream.
/// If the object is a stream, it will return its dictionary as well as the stream
/// itself.
pub fn dict_or_stream<'a, 'b>(
    obj: &'b Object<'a>,
) -> Option<(&'b Dict<'a>, Option<&'b Stream<'a>>)> {
    if let Some(stream) = obj.cast_ref::<Stream<'a>>() {
        Some((stream.dict(), Some(stream)))
    } else {
        obj.cast_ref::<Dict<'a>>().map(|dict| (dict, None))
    }
}

mod macros {
    macro_rules! object {
        ($t:ident $(<$l:lifetime>),*, $s:ident) => {
            impl<'a> TryFrom<Object<'a>> for $t$(<$l>),* {
                type Error = ();

                fn try_from(value: Object<'a>) -> core::result::Result<Self, Self::Error> {
                    match value {
                        Object::$s(b) => Ok(b),
                        _ => Err(()),
                    }
                }
            }

            impl<'a> crate::object::ObjectLike<'a> for $t$(<$l>),* {}

            impl<'a> crate::object::ObjectRefLike<'a> for $t$(<$l>),* {
                fn cast_ref<'b>(obj: &'b crate::object::Object<'a>) -> Option<&'b Self> {
                    match obj {
                        crate::object::Object::$s(b) => Some(b),
                        _ => None,
                    }
                }
            }
        };
    }

    pub(crate) use object;
}

#[cfg(test)]
mod tests {
    use crate::object::{
        Array, Dict, MaybeRef, Name, Null, Number, ObjRef, Object, Stream, String,
    };
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};
    use crate::trivia::is_regular_character;

    fn object_impl(data: &[u8]) -> Option<Object<'_>> {
        let mut r = Reader::new(data);
        r.read_with_context::<Object<'_>>(&ReaderContext::dummy())
    }

    #[test]
    fn deeply_nested_arrays() {
        let data = format!("{}0{}", "[".repeat(5000), "]".repeat(5000));
        assert!(object_impl(data.as_bytes()).is_none());
    }

    #[test]
    fn deeply_nested_dictionaries() {
        let data = format!("{}0{}", "<< /X ".repeat(5000), " >>".repeat(5000));
        assert!(object_impl(data.as_bytes()).is_none());
    }

    #[test]
    fn null() {
        assert!(matches!(object_impl(b"null").unwrap(), Object::Null(_)));
    }

    #[test]
    fn bool() {
        assert!(matches!(object_impl(b"true").unwrap(), Object::Boolean(_)));
    }

    #[test]
    fn number() {
        assert!(matches!(object_impl(b"34.5").unwrap(), Object::Number(_)));
    }

    #[test]
    fn string_1() {
        assert!(matches!(object_impl(b"(Hi)").unwrap(), Object::String(_)));
    }

    #[test]
    fn string_2() {
        assert!(matches!(object_impl(b"<34>").unwrap(), Object::String(_)));
    }

    #[test]
    fn name() {
        assert!(matches!(object_impl(b"/Name").unwrap(), Object::Name(_)));
    }

    #[test]
    fn dict() {
        assert!(matches!(
            object_impl(b"<</Entry 45>>").unwrap(),
            Object::Dict(_)
        ));
    }

    #[test]
    fn array() {
        assert!(matches!(object_impl(b"[45]").unwrap(), Object::Array(_)));
    }

    #[test]
    fn stream() {
        assert!(matches!(
            object_impl(b"<< /Length 3 >> stream\nabc\nendstream").unwrap(),
            Object::Stream(_)
        ));
    }

    #[test]
    fn display() {
        let cases: &[(&[u8], &str)] = &[
            (b"null", "null"),
            (b"true", "true"),
            (b"34.5", "34.5"),
            (b"(Hi)", "\"Hi\""),
            (b"/Name", "/Name"),
            (b"[45]", "[45]"),
            (b"<</Entry 45>>", "<< /Entry 45 >>"),
            (b"<< /Length 3 >> stream\nabc\nendstream", "Stream (len: 3)"),
        ];

        for (input, expected) in cases {
            assert_eq!(format!("{}", object_impl(input).unwrap()), *expected);
        }
    }

    #[test]
    fn token_that_starts_like_a_number_ends_where_skip_ends_it() {
        // Every token of up to five of these characters that starts like a number,
        // read as an array item and as a dictionary value. Skipping and reading it must
        // end at the same place, so the next item is still read, and reading it as a
        // number must agree with reading it as an object.
        let mut tokens = vec![];
        let mut longer: Vec<Vec<u8>> = vec![vec![]];
        for _ in 0..5 {
            longer = longer
                .iter()
                .flat_map(|token| b".+-05ef".map(|c| [token.as_slice(), &[c]].concat()))
                .collect();
            tokens.extend(longer.iter().filter(|t| b".+-05".contains(&t[0])).cloned());
        }

        for token in tokens {
            let token = core::str::from_utf8(&token).unwrap();
            let array = format!("[{token} 7]");
            let Some(Object::Array(array)) = object_impl(array.as_bytes()) else {
                panic!("{array} isn't an array");
            };
            let items = array.iter::<Object<'_>>().collect::<Vec<_>>();
            assert_eq!(items.len(), 2, "[{token} 7]");
            assert!(
                matches!(items[1], Object::Number(n) if n.as_i64() == 7),
                "[{token} 7]"
            );
            let first = items[0].clone();
            assert_eq!(
                array.flex_iter().next::<i32>(),
                first.clone().into_i32(),
                "[{token} 7]"
            );
            assert_eq!(
                array.flex_iter().next::<f32>(),
                first.into_f32(),
                "[{token} 7]"
            );

            let dict = format!("<< /A {token} /B 7 >>");
            let Some(Object::Dict(dict)) = object_impl(dict.as_bytes()) else {
                panic!("{dict} isn't a dictionary");
            };
            assert_eq!(dict.len(), 2, "<< /A {token} /B 7 >>");
            assert_eq!(dict.get::<i32>(b"B"), Some(7), "<< /A {token} /B 7 >>");
            let value = dict.get::<Object<'_>>(b"A").unwrap();
            assert_eq!(
                dict.get::<i32>(b"A"),
                value.clone().into_i32(),
                "<< /A {token} /B 7 >>"
            );
            assert_eq!(
                dict.get::<f32>(b"A"),
                value.into_f32(),
                "<< /A {token} /B 7 >>"
            );
        }
    }

    // Arrays and dictionaries are validated by skipping their items, and their items are
    // later read one at a time from where validation found them. For many generated
    // containers that validation accepts, this checks that reading each item ends where
    // skipping it ended, that it reads a reference only where validation skipped one,
    // and that reading it as `i32` or `f32` agrees with reading it as an object. It also
    // checks that each item reads as the same reference or number as content streams read
    // there, unless one of the numbers they read runs into a regular character.
    #[test]
    fn reading_container_items_agrees_with_skipping_them() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let mut failures = vec![];
        let mut accepted = 0;

        for i in 0..100_000 {
            let mut input = vec![];
            if i % 2 == 0 {
                gen_array(&mut rng, 0, &mut input);
            } else {
                gen_dict(&mut rng, 0, &mut input);
            }

            let Some(object) = object_impl(&input) else {
                continue;
            };
            accepted += 1;
            let mut errors = vec![];
            check_object(&object, &mut errors);
            if !errors.is_empty() {
                failures.push(format!("{}: {}", BStr(&input), errors.join("; ")));
            }
        }

        assert!(
            accepted > 50_000,
            "only {accepted} containers were accepted"
        );
        assert!(
            failures.is_empty(),
            "{} of {accepted} containers disagree, for example:\n{}",
            failures.len(),
            failures[..failures.len().min(10)].join("\n")
        );
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }

        fn pick<'a>(&mut self, items: &[&'a [u8]]) -> &'a [u8] {
            items[self.below(items.len())]
        }
    }

    fn gen_items(rng: &mut Rng, depth: usize, count: usize, out: &mut Vec<u8>) {
        for i in 0..count {
            // Leave out the white space now and then, so that tokens run together.
            if i > 0 && rng.below(3) != 0 {
                out.push(b' ');
            }
            match rng.below(if depth < 2 { 10 } else { 8 }) {
                0..=2 => {
                    for _ in 0..=rng.below(3) {
                        out.push(b"0158.+-e"[rng.below(8)]);
                    }
                }
                3 | 4 => out.extend_from_slice(rng.pick(&[
                    b"0",
                    b"8",
                    b"12",
                    b"-3",
                    b"4.5",
                    b"0.0",
                    b"2147483647",
                    b"2147483648",
                    b"-2147483649",
                    b"12345678901234567890123",
                ])),
                5 => out.push(b'R'),
                // No `stream` keyword: `Object::read` reads a dictionary followed by it as
                // a stream, which skipping doesn't.
                6 => out.extend_from_slice(rng.pick(&[
                    b"foo", b"e", b"null", b"nul", b"true", b"false", b"obj", b"/A", b"/Name",
                    b"(s)", b"(a(b)c)", b"(\\))", b"<0a>", b"%c\n",
                ])),
                7 => out.extend_from_slice(rng.pick(&[b"/A", b"/B"])),
                8 => gen_array(rng, depth + 1, out),
                _ => gen_dict(rng, depth + 1, out),
            }
        }
    }

    fn gen_array(rng: &mut Rng, depth: usize, out: &mut Vec<u8>) {
        out.push(b'[');
        let count = rng.below(6);
        gen_items(rng, depth, count, out);
        out.push(b']');
    }

    fn gen_dict(rng: &mut Rng, depth: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(b"<<");
        for _ in 0..rng.below(4) {
            out.push(b' ');
            out.extend_from_slice(rng.pick(&[b"/A", b"/B", b"/C"]));
            out.push(b' ');
            let count = 1 + rng.below(3);
            gen_items(rng, depth, count, out);
        }
        out.extend_from_slice(b" >>");
    }

    struct BStr<'a>(&'a [u8]);

    impl core::fmt::Display for BStr<'_> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(core::str::from_utf8(self.0).unwrap())
        }
    }

    fn check_object(object: &Object<'_>, errors: &mut Vec<std::string::String>) {
        match object {
            Object::Array(array) => check_array(array, errors),
            Object::Dict(dict) => check_dict(dict, errors),
            _ => {}
        }
    }

    // Skips one item the way `Array::skip` and `Dict::skip` do outside of content streams,
    // returning where it ends and whether it was skipped as a reference.
    fn skip_item(r: &mut Reader<'_>) -> (usize, bool) {
        let is_ref = r.clone().skip::<ObjRef>(false).is_some();
        r.skip::<MaybeRef<Object<'_>>>(false)
            .expect("the container was validated");
        (r.offset(), is_ref)
    }

    // Where the next item starts: reading a dictionary also reads the white space after it,
    // to look for a stream.
    fn next_item(data: &[u8], end: usize) -> usize {
        let mut r = Reader::new(data);
        r.jump(end);
        r.skip_white_spaces_and_comments();
        r.offset()
    }

    // Reads the item at `start` of `data` the way `ArrayIter`, `FlexArrayIter` and
    // `Dict::get_raw` do, as an object, an `i32` and an `f32`, and checks it against
    // what skipping it found.
    fn check_item<'a>(
        data: &'a [u8],
        start: usize,
        (end, is_ref): (usize, bool),
        ctx: &ReaderContext<'a>,
        errors: &mut Vec<std::string::String>,
    ) {
        let skipped = BStr(&data[start..end]);
        let mut r = Reader::new(&data[start..]);
        let Some(object) = r.read_with_context::<MaybeRef<Object<'_>>>(ctx) else {
            errors.push(format!("`{skipped}` can't be read"));
            return;
        };
        if next_item(data, start + r.offset()) != next_item(data, end) {
            errors.push(format!(
                "skipping `{skipped}` and reading `{}` end differently",
                BStr(&data[start..start + r.offset()])
            ));
        }
        if matches!(object, MaybeRef::Ref(_)) != is_ref {
            errors.push(format!("`{skipped}` reads as {object:?}, is_ref: {is_ref}"));
        }
        let read = match &object {
            MaybeRef::Ref(obj_ref) => Lenient::Ref(*obj_ref),
            MaybeRef::NotRef(Object::Number(number)) => Lenient::Number(*number),
            MaybeRef::NotRef(_) => Lenient::Other,
        };
        let (lenient, lenient_end, ended) = read_leniently(&data[start..]);
        let same = read == lenient && (read == Lenient::Other || lenient_end == r.offset());
        // Where a number read leniently runs into a regular character, the item may read
        // differently, but not as a reference.
        if !same && (ended || matches!(read, Lenient::Ref(_))) {
            errors.push(format!(
                "`{skipped}` reads as {object:?}, in a content stream as {lenient:?}"
            ));
        }

        let value = match &object {
            MaybeRef::Ref(_) => None,
            MaybeRef::NotRef(object) => Some(object),
        };
        let mut r = Reader::new(&data[start..]);
        match r.read_with_context::<MaybeRef<i32>>(ctx) {
            Some(MaybeRef::Ref(_)) if !is_ref => {
                errors.push(format!("`{skipped}` reads as a reference to i32"));
            }
            Some(MaybeRef::NotRef(n)) if value.and_then(|v| v.clone().into_i32()) != Some(n) => {
                errors.push(format!(
                    "`{skipped}` reads as i32 {n}, as an object {value:?}"
                ));
            }
            None if value.and_then(|v| v.clone().into_i32()).is_some() || is_ref => {
                errors.push(format!("`{skipped}` can't be read as i32"));
            }
            Some(_) if next_item(data, start + r.offset()) != next_item(data, end) => {
                errors.push(format!("`{skipped}` read as i32 ends elsewhere"));
            }
            _ => {}
        }
        let mut r = Reader::new(&data[start..]);
        match r.read_with_context::<MaybeRef<f32>>(ctx) {
            Some(MaybeRef::Ref(_)) if !is_ref => {
                errors.push(format!("`{skipped}` reads as a reference to f32"));
            }
            Some(MaybeRef::NotRef(n)) if value.and_then(|v| v.clone().into_f32()) != Some(n) => {
                errors.push(format!(
                    "`{skipped}` reads as f32 {n}, as an object {value:?}"
                ));
            }
            None if value.and_then(|v| v.clone().into_f32()).is_some() || is_ref => {
                errors.push(format!("`{skipped}` can't be read as f32"));
            }
            Some(_) if next_item(data, start + r.offset()) != next_item(data, end) => {
                errors.push(format!("`{skipped}` read as f32 ends elsewhere"));
            }
            _ => {}
        }
    }

    #[derive(Debug, PartialEq)]
    enum Lenient {
        Ref(ObjRef),
        Number(Number),
        Other,
    }

    // Reads the item at the start of `data` as a reference or a number the way content
    // streams read them, which doesn't require a number to end at white space or a
    // delimiter. Returns what it read, where it ended, and whether each number did end so.
    fn read_leniently(data: &[u8]) -> (Lenient, usize, bool) {
        let mut ended = true;
        let mut r = Reader::new(data);
        if let Some(obj_ref) = lenient_ref(&mut r, &mut ended) {
            return (Lenient::Ref(obj_ref), r.offset(), ended);
        }

        let mut ended = true;
        let mut r = Reader::new(data);
        if matches!(data.first(), Some(b'.' | b'+' | b'-' | b'0'..=b'9'))
            && let Some(number) = lenient_number(&mut r, &mut ended)
        {
            return (Lenient::Number(number), r.offset(), ended);
        }

        (Lenient::Other, 0, true)
    }

    fn lenient_ref(r: &mut Reader<'_>, ended: &mut bool) -> Option<ObjRef> {
        let obj_number = lenient_number(r, ended)?.as_i64().try_into().ok()?;
        r.skip_white_spaces();
        let gen_number = lenient_number(r, ended)?.as_i64().try_into().ok()?;
        r.skip_white_spaces();
        r.forward_tag(b"R")?;

        Some(ObjRef::new(obj_number, gen_number))
    }

    fn lenient_number(r: &mut Reader<'_>, ended: &mut bool) -> Option<Number> {
        let number = r.read_with_context::<Number>(ReaderContext::dummy_ref())?;
        *ended &= !r.peek_byte().is_some_and(is_regular_character);

        Some(number)
    }

    fn check_array(array: &Array<'_>, errors: &mut Vec<std::string::String>) {
        let data = array.data();
        let ctx = ReaderContext::dummy();
        let mut r = Reader::new(data);
        let mut skipped = vec![];
        loop {
            r.skip_white_spaces_and_comments();
            if r.at_end() {
                break;
            }
            let start = r.offset();
            let item = skip_item(&mut r);
            check_item(data, start, item, &ctx, errors);
            skipped.push(item.1);
        }

        let read = array.raw_iter().collect::<Vec<_>>();
        if read.len() != skipped.len() {
            errors.push(format!(
                "[{}] has {} items when skipped and {} when read: {read:?}",
                BStr(data),
                skipped.len(),
                read.len()
            ));
        }
        for (item, is_ref) in read.iter().zip(skipped) {
            if matches!(item, MaybeRef::Ref(_)) != is_ref {
                errors.push(format!("{item:?} in [{}], is_ref: {is_ref}", BStr(data)));
            }
            if let MaybeRef::NotRef(object) = item {
                check_object(object, errors);
            }
        }
    }

    fn check_dict(dict: &Dict<'_>, errors: &mut Vec<std::string::String>) {
        let data = dict.data();
        let ctx = ReaderContext::dummy();
        let mut r = Reader::new(data);
        r.forward_tag(b"<<").unwrap();
        let mut entries = std::collections::HashMap::new();
        loop {
            r.skip_white_spaces_and_comments();
            if r.forward_tag(b">>").is_some() {
                break;
            }
            let Some(key) = r.skip::<Name<'_>>(false) else {
                r.skip::<Object<'_>>(false)
                    .expect("the container was validated");
                continue;
            };
            r.skip_white_spaces_and_comments();
            let start = r.offset();
            let value = skip_item(&mut r);
            check_item(data, start, value, &ctx, errors);
            entries.insert(&key[1..], value.1);
        }

        if dict.len() != entries.len() {
            errors.push(format!(
                "{} has {} keys when skipped and {} when read",
                BStr(data),
                entries.len(),
                dict.len()
            ));
        }
        for (key, is_ref) in entries {
            let Some(value) = dict.get_raw::<Object<'_>>(key) else {
                errors.push(format!("/{} in {} can't be read", BStr(key), BStr(data)));
                continue;
            };
            if matches!(value, MaybeRef::Ref(_)) != is_ref {
                errors.push(format!(
                    "/{} in {} reads as {value:?}",
                    BStr(key),
                    BStr(data)
                ));
            }
            if let MaybeRef::NotRef(object) = value {
                if dict.get::<i32>(key) != object.clone().into_i32() {
                    errors.push(format!("/{} in {} as i32", BStr(key), BStr(data)));
                }
                if dict.get::<f32>(key) != object.clone().into_f32() {
                    errors.push(format!("/{} in {} as f32", BStr(key), BStr(data)));
                }
                check_object(&object, errors);
            }
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn object_sizes() {
        assert_eq!(size_of::<Object<'_>>(), 40);
        assert_eq!(size_of::<Array<'_>>(), 32);
        assert_eq!(size_of::<Dict<'_>>(), 8);
        assert_eq!(size_of::<Name<'_>>(), 32);
        assert_eq!(size_of::<Null>(), 0);
        assert_eq!(size_of::<Number>(), 16);
        assert_eq!(size_of::<Stream<'_>>(), 24);
        assert_eq!(size_of::<String<'_>>(), 32);
    }
}
