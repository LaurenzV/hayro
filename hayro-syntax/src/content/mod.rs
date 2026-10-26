/*!
PDF content operators.

This module provides facilities to read and interpret PDF content streams using
high-level types.

```
use hayro_syntax::object::Number;
use hayro_syntax::content::*;
use hayro_syntax::content::ops::*;

let content_stream = b"1 0 0 -1 0 200 cm
0 1.0 0 rg
0 0 m
200 0 l
200 200 l
0 200 l
h
f";

let mut iter = TypedIter::new(content_stream);
assert!(matches!(iter.next(), Some(TypedInstruction::Transform(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::NonStrokeColorDeviceRgb(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::MoveTo(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::LineTo(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::ClosePath(_))));
assert!(matches!(iter.next(), Some(TypedInstruction::FillPathNonZero(_))));
```
*/

#[allow(missing_docs)]
pub mod ops;

use crate::content::ops::TypedInstruction;
use crate::object;
use crate::object::dict::InlineImageDict;
use crate::object::name::{Name, skip_name_like};
use crate::object::{Array, Number, Object, Stream};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::{is_regular_character, is_white_space_character};
use crate::util::{find_needle, findr_needle};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use smallvec::SmallVec;

// `scn` with 32 DeviceN components and a pattern name.
const OPERANDS_THRESHOLD: usize = 33;
const INLINE_COLOR_COMPONENTS: usize = 4;

impl Debug for Operator<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl Display for Operator<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

/// A content stream operator.
#[derive(Clone, PartialEq)]
pub struct Operator<'a>(Name<'a>);

impl Deref for Operator<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl Skippable for Operator<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        skip_name_like(r, false).map(|_| ())
    }
}

impl<'a> Readable<'a> for Operator<'a> {
    fn read(r: &mut Reader<'a>, _: &ReaderContext<'a>) -> Option<Self> {
        let start = r.offset();
        let mut escaped = false;
        while let Some(b) = r.peek_byte().filter(|b| is_regular_character(*b)) {
            escaped |= b == b'#';
            r.forward();
        }

        let data = r.range(start..r.offset())?;
        if data.is_empty() {
            return None;
        }

        let name = if escaped {
            Name::new_escaped(data)?
        } else {
            Name::new_unescaped(data)
        };

        Some(Self(name))
    }
}

/// An iterator over operators in the PDF content streams, providing raw access to the instructions.
#[derive(Debug, Clone)]
pub struct UntypedIter<'a> {
    reader: Reader<'a>,
    stack: Stack<'a>,
    operator: Option<Operator<'a>>,
}

impl<'a> UntypedIter<'a> {
    /// Create a new untyped iterator.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
            stack: Stack::new(),
            operator: None,
        }
    }

    /// Create a new empty untyped iterator.
    pub fn empty() -> Self {
        Self {
            reader: Reader::new(&[]),
            stack: Stack::new(),
            operator: None,
        }
    }

    /// Return the next instruction.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Instruction<'_, 'a>> {
        self.stack.clear();

        let operator = read_instruction(&mut self.reader, &mut self.stack)?;
        self.operator = Some(operator);
        Some(Instruction {
            operands: &self.stack,
            operator: self.operator.as_ref().unwrap(),
        })
    }
}

fn read_instruction<'a>(r: &mut Reader<'a>, stack: &mut Stack<'a>) -> Option<Operator<'a>> {
    // Keeps the read position in a register; must be written back before calling out.
    let mut cur = r.clone();

    loop {
        cur.skip_white_spaces_and_comments();

        let Some(first) = cur.peek_byte() else {
            *r = cur;
            return None;
        };

        if matches!(first, b'.' | b'+' | b'-' | b'0'..=b'9') {
            let start = cur.offset();
            match Number::read_simple(&mut cur) {
                Some(number) => {
                    if stack.push(Object::Number(number)).is_none() {
                        *r = cur;
                        return None;
                    }
                    continue;
                }
                None => cur.jump(start),
            }
        }

        match first {
            // I believe booleans/null never appear as an operator?
            b'.' | b'+' | b'-' | b'0'..=b'9' | b'/' | b'[' | b'<' | b'(' => {
                *r = cur;
                read_operand(r, stack)?;
                cur = r.clone();
            }
            _ => {
                *r = cur;
                let Some(operator) = r.read_without_context::<Operator<'a>>() else {
                    r.jump_to_end();
                    return None;
                };

                // Inline images need special casing...
                if operator.as_ref() == b"BI" {
                    read_inline_image(r, stack)?;
                }

                return Some(operator);
            }
        }
    }
}

fn read_operand<'a>(r: &mut Reader<'a>, stack: &mut Stack<'a>) -> Option<()> {
    // See issue 994. In all sane scenarios, if the next byte is a number
    // it has to be an operand (a number). However, it's possible that
    // the number is followed by a regular character, in which case it
    // should behave more like an operator (even though there exists
    // no operator that starts with a number). In order to preserve
    // similar behavior to Acrobat and Chromium, we try to consume
    // such an operator and then simply skip it.
    if let Some(object) = r.read_without_context::<Object<'a>>() {
        stack.push(object)
    } else if r.read_without_context::<Operator<'a>>().is_some() {
        stack.clear();
        Some(())
    } else {
        None
    }
}

fn read_inline_image<'a>(r: &mut Reader<'a>, stack: &mut Stack<'a>) -> Option<()> {
    // The ID operator will already be consumed by this.
    let inline_dict = r.read_without_context::<InlineImageDict<'_>>()?;
    let dict = inline_dict.get_dict().clone();

    // One whitespace after "ID".
    r.read_white_space()?;

    let stream_data = r.tail()?;
    let mut search = EndSearch::new(stream_data);
    let mut pos = 0;

    while let Some(ei) = find_needle(&stream_data[pos..], b"EI").map(|ei| pos + ei) {
        // If the following character is not a whitespace character, then we are in a ASCII-85 stream.
        if stream_data
            .get(ei + 2)
            .is_some_and(|b| !is_white_space_character(*b))
        {
            pos = ei + 3;

            continue;
        }

        if search.is_end(ei) {
            stack.push(Object::Stream(Stream::new(&stream_data[..ei], dict)))?;

            r.read_bytes(ei + 2)?;
            r.skip_white_spaces();

            return Some(());
        }

        pos = ei + 2;
    }

    r.read_bytes(pos)?;

    Some(())
}

// Reuse lookahead between successive `EI` candidates.
struct EndSearch<'a> {
    data: &'a [u8],
    // None means unsearched; Some(None) means no later inline image.
    next_bi: Option<Option<usize>>,
    // Cache the last `EI` with the `BI` that bounds the search.
    last_ei: Option<(Option<usize>, Option<usize>)>,
    binary: BinaryTracker,
}

impl<'a> EndSearch<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            next_bi: None,
            last_ei: None,
            binary: BinaryTracker::default(),
        }
    }

    fn is_end(&mut self, ei: usize) -> bool {
        // Note that there is a possibility that the encoded stream data
        // contains the "EI" operator as part of the data, in which case we
        // cannot confidently know whether we have hit the actual end of the
        // stream. See also <https://github.com/pdf-association/pdf-issues/issues/543>
        // PDF 2.0 does have a `/Length` attribute we can read, but since it's relatively
        // new we don't bother trying to read it.
        let tail = ei + 2;
        let bi = self.next_bi(tail);

        let Some(last_ei) = self.last_ei_before(tail, bi) else {
            return true;
        };

        // Scan only to the next `EI` until the content check succeeds.
        let next_ei = find_needle(&self.data[tail..], b"EI").map_or(last_ei, |ei| tail + ei);
        if self.binary.found_before(self.data, tail, next_ei) {
            return false;
        }

        // A cached rejection avoids repeating the content check.
        if self.binary.has_scanned(last_ei) && self.binary.found_before(self.data, tail, last_ei) {
            return false;
        }

        // A later `EI` can belong to a string in the content after this image.
        if !starts_with_content(&self.data[tail..]) {
            return false;
        }

        !self.binary.found_before(self.data, tail, last_ei)
    }

    fn next_bi(&mut self, from: usize) -> Option<usize> {
        if let Some(bi) = self.next_bi
            && bi.is_none_or(|bi| bi >= from)
        {
            return bi;
        }

        let mut pos = from;
        let bi = loop {
            let Some(bi) = find_needle(&self.data[pos..], b"BI").map(|bi| pos + bi) else {
                break None;
            };

            let mut r = Reader::new(&self.data[bi + 2..]);
            if r.read_without_context::<InlineImageDict<'_>>().is_some() {
                break Some(bi);
            }

            pos = bi + 1;
        };

        self.next_bi = Some(bi);
        bi
    }

    fn last_ei_before(&mut self, from: usize, bi: Option<usize>) -> Option<usize> {
        let ei = match self.last_ei {
            Some((searched_bi, ei)) if searched_bi == bi => ei,
            _ => {
                let end = bi.unwrap_or(self.data.len());
                let ei = findr_needle(&self.data[from..end], b"EI").map(|ei| from + ei);
                self.last_ei = Some((bi, ei));
                ei
            }
        };

        ei.filter(|ei| *ei >= from)
    }
}

fn starts_with_content(data: &[u8]) -> bool {
    let mut iter = TypedIter::new(data);
    let mut counter = 0;

    while let Some(op) = iter.next() {
        // If we have read more than 20 valid operators, it should be
        // safe to assume that we are in a content stream, so abort
        // early. The only situation where this could reasonably
        // be violated is if we have 20 subsequent instances of
        // q/Q in the image data, which seems very unlikely.
        if counter >= 20 {
            return true;
        }

        if matches!(
            op,
            TypedInstruction::NextLineAndShowText(_)
                | TypedInstruction::ShowText(_)
                | TypedInstruction::ShowTexts(_)
                | TypedInstruction::ShowTextWithParameters(_)
        ) {
            return true;
        }

        counter += 1;
    }

    false
}

// Content streams only hold non-ASCII bytes within strings, so one outside
// of them means that we are still within binary image data.
// Reuse the scan unless a later candidate starts within a string it has seen.
#[derive(Default)]
struct BinaryTracker {
    start: usize,
    pos: usize,
    string_depth: usize,
    escaped: bool,
    string_start: usize,
    // Opening and closing parentheses of completed strings.
    strings: VecDeque<(usize, usize)>,
    found: bool,
}

impl BinaryTracker {
    /// Whether a non-ASCII byte outside of strings lies between `start`
    /// and `end`. `start` never decreases between calls.
    fn found_before(&mut self, data: &[u8], start: usize, end: usize) -> bool {
        if start != self.start {
            self.move_start(data, start);
        }

        if self.found || end <= self.pos {
            return self.found && self.pos < end;
        }

        self.scan(data, end);
        self.found
    }

    fn has_scanned(&self, end: usize) -> bool {
        self.found || end <= self.pos
    }

    fn move_start(&mut self, data: &[u8], start: usize) {
        if !self.found && self.pos < start {
            self.scan(data, start);
        }

        // A binary byte before `start` invalidates the cached scan.
        let lost = self.found && self.pos < start;

        if lost || self.in_string_at(start) {
            *self = Self {
                start,
                pos: start,
                ..Self::default()
            };
        } else {
            self.start = start;
        }
    }

    fn in_string_at(&mut self, start: usize) -> bool {
        if self.string_depth > 0 && self.string_start < start && start <= self.pos {
            return true;
        }

        while self
            .strings
            .front()
            .is_some_and(|(_, close)| *close < start)
        {
            self.strings.pop_front();
        }

        self.strings.front().is_some_and(|(open, _)| *open < start)
    }

    fn scan(&mut self, data: &[u8], end: usize) {
        for (i, &b) in data[self.pos..end].iter().enumerate() {
            if self.string_depth > 0 {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'(' {
                    self.string_depth += 1;
                } else if b == b')' {
                    self.string_depth -= 1;

                    if self.string_depth == 0 {
                        self.strings.push_back((self.string_start, self.pos + i));
                    }
                }
            } else if !b.is_ascii() {
                self.found = true;
                self.pos += i;
                return;
            } else if b == b'(' {
                self.string_depth = 1;
                self.string_start = self.pos + i;
            }
        }

        self.pos = end;
    }
}

/// An iterator over PDF content streams that provide access to the instructions
/// in a typed fashion.
#[derive(Clone)]
pub struct TypedIter<'a> {
    untyped: UntypedIter<'a>,
}

impl<'a> TypedIter<'a> {
    /// Create a new typed iterator.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            untyped: UntypedIter::new(data),
        }
    }

    pub(crate) fn from_untyped(untyped: UntypedIter<'a>) -> Self {
        Self { untyped }
    }

    /// Return the next typed instruction.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<TypedInstruction<'_, 'a>> {
        let op = self.untyped.next()?;
        // TODO: Explore whether dispatching can be made more efficient.
        match TypedInstruction::dispatch(&op) {
            Some(op) => Some(op),
            // In case this returns `None`, the content stream is invalid. In case a path-drawing
            // operator was used, let's abort completely, otherwise we might end up drawing random stuff.
            // However, for other operators it could be worth it to just skip it but keep attempting
            // to read other content operators.
            None => {
                if [
                    &b"m"[..],
                    &b"l"[..],
                    &b"c"[..],
                    &b"v"[..],
                    &b"y"[..],
                    &b"h"[..],
                    &b"re"[..],
                ]
                .contains(&op.operator.0.deref())
                {
                    None
                } else {
                    Some(TypedInstruction::Fallback(op.operator))
                }
            }
        }
    }
}

/// An instruction (= operator and its operands) in a content stream.
#[derive(Debug)]
pub struct Instruction<'b, 'a> {
    /// The stack containing the operands.
    pub operands: &'b Stack<'a>,
    /// The actual operator.
    pub operator: &'b Operator<'a>,
}

impl<'b, 'a> Instruction<'b, 'a> {
    /// An iterator over the operands of the instruction.
    pub fn operands(&self) -> OperandIterator<'b, 'a> {
        OperandIterator::new(self.operands)
    }
}

/// A stack holding the arguments of an operator.
#[derive(Clone, PartialEq)]
pub struct Stack<'a> {
    // TODO: Explore using an object pool to avoid repeatedly
    // allocating/deallocating objects.
    data: Vec<Object<'a>>,
}

impl<'a> Default for Stack<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Stack<'a> {
    /// Create a new, empty stack.
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn push(&mut self, operand: Object<'a>) -> Option<()> {
        if self.data.len() >= OPERANDS_THRESHOLD {
            return None;
        }

        self.data.push(operand);
        Some(())
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn as_slice(&self) -> &[Object<'a>] {
        &self.data
    }

    fn get<'b, T>(&'b self, index: usize) -> Option<T>
    where
        T: Operand<'b, 'a>,
    {
        self.as_slice().get(index).and_then(T::from_object)
    }

    fn get_all<'b, T>(&'b self) -> Option<SmallVec<[T; INLINE_COLOR_COMPONENTS]>>
    where
        T: Operand<'b, 'a>,
    {
        let mut operands = SmallVec::new();

        for op in self.as_slice() {
            let converted = T::from_object(op)?;
            operands.push(converted);
        }

        Some(operands)
    }
}

impl Debug for Stack<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

trait Operand<'b, 'a>: Sized {
    fn from_object(object: &'b Object<'a>) -> Option<Self>;
}

impl<'b, 'a> Operand<'b, 'a> for Number {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        match object {
            Object::Number(n) => Some(*n),
            _ => None,
        }
    }
}

impl<'b, 'a> Operand<'b, 'a> for &'b object::String<'a> {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        match object {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
}

impl<'b, 'a> Operand<'b, 'a> for &'b Name<'a> {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        match object {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }
}

impl<'b, 'a> Operand<'b, 'a> for &'b Array<'a> {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        match object {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }
}

impl<'b, 'a> Operand<'b, 'a> for &'b Stream<'a> {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        match object {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }
}

impl<'b, 'a> Operand<'b, 'a> for &'b Object<'a> {
    fn from_object(object: &'b Object<'a>) -> Option<Self> {
        Some(object)
    }
}

/// An iterator over the operands of an operator.
pub struct OperandIterator<'b, 'a> {
    stack: &'b Stack<'a>,
    cur_index: usize,
}

impl<'b, 'a> OperandIterator<'b, 'a> {
    fn new(stack: &'b Stack<'a>) -> Self {
        Self {
            stack,
            cur_index: 0,
        }
    }
}

impl<'b, 'a> Iterator for OperandIterator<'b, 'a> {
    type Item = &'b Object<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.stack.as_slice().get(self.cur_index) {
            self.cur_index += 1;

            Some(item)
        } else {
            None
        }
    }
}

pub(crate) trait OperatorTrait<'b, 'a>: Sized {
    fn from_stack(stack: &'b Stack<'a>) -> Option<Self>;
}

mod macros {
    macro_rules! display_op {
        ($t:ident $(<$($l:lifetime),+>)?;) => {
            impl$(<$($l),+>)? core::fmt::Display for $t$(<$($l),+>)? {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    f.write_str(stringify!($t))
                }
            }
        };
        ($t:ident $(<$($l:lifetime),+>)?; $first:tt $(, $field:tt)*) => {
            impl$(<$($l),+>)? core::fmt::Display for $t$(<$($l),+>)? {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    f.write_str(concat!(stringify!($t), "("))?;
                    core::fmt::Display::fmt(&self.$first, f)?;
                    $(
                        f.write_str(" ")?;
                        core::fmt::Display::fmt(&self.$field, f)?;
                    )*
                    f.write_str(")")
                }
            }
        };
    }

    macro_rules! display_variadic_op {
        ($t:ident $(<$($l:lifetime),+>)?) => {
            impl$(<$($l),+>)? core::fmt::Display for $t$(<$($l),+>)? {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    f.write_str(stringify!($t))?;
                    if let Some((first, rest)) = self.0.split_first() {
                        f.write_str("(")?;
                        core::fmt::Display::fmt(first, f)?;
                        for value in rest {
                            f.write_str(" ")?;
                            core::fmt::Display::fmt(value, f)?;
                        }
                        f.write_str(")")?;
                    }
                    Ok(())
                }
            }
        };
    }

    macro_rules! op_impl {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr, $n:expr, |$stack:ident : $stack_ty:ty| $body:block) => {
            impl<'b, 'a> OperatorTrait<'b, 'a> for $t$(<$($l),+>)? {
                #[inline(always)]
                fn from_stack($stack: $stack_ty) -> Option<Self> {
                    $body
                }
            }

            impl<'b, 'a> From<$t$(<$($l),+>)?> for TypedInstruction<'b, 'a> {
                fn from(value: $t$(<$($l),+>)?) -> Self {
                    TypedInstruction::$t(value)
                }
            }

            impl<'b, 'a> TryFrom<TypedInstruction<'b, 'a>> for $t$(<$($l),+>)? {
                type Error = ();

                fn try_from(value: TypedInstruction<'b, 'a>) -> core::result::Result<Self, Self::Error> {
                    match value {
                        TypedInstruction::$t(e) => Ok(e),
                        _ => Err(())
                    }
                }
            }
        };
    }

    // The `shift` parameter will always be 0 in valid PDFs. The purpose of the parameter is
    // so that in case there are garbage operands in the content stream, we prefer to use
    // the operands that are closer to the operator instead of the values at the bottom
    // of the stack.

    macro_rules! op0 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?;);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 0, |_stack: &'b Stack<'a>| {
                Some(Self)
            });
        }
    }

    macro_rules! op1 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?; 0);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 1, |stack: &'b Stack<'a>| {
                let shift = stack.len().saturating_sub(1);
                Some(Self(stack.get(0 + shift)?))
            });
        }
    }

    macro_rules! op_all {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_variadic_op!($t$(<$($l),+>)?);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, u8::MAX as usize, |stack: &'b Stack<'a>| {
                Some(Self(stack.get_all()?))
            });
        }
    }

    macro_rules! op2 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?; 0, 1);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 2, |stack: &'b Stack<'a>| {
                let shift = stack.len().saturating_sub(2);
                Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?))
            });
        }
    }

    macro_rules! op3 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?; 0, 1, 2);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 3, |stack: &'b Stack<'a>| {
                let shift = stack.len().saturating_sub(3);
                Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
                stack.get(2 + shift)?))
            });
        }
    }

    macro_rules! op4 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?; 0, 1, 2, 3);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 4, |stack: &'b Stack<'a>| {
               let shift = stack.len().saturating_sub(4);
            Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
            stack.get(2 + shift)?, stack.get(3 + shift)?))
            });
        }
    }

    macro_rules! op6 {
        ($t:ident $(<$($l:lifetime),+>)?, $e:expr) => {
            crate::content::macros::display_op!($t$(<$($l),+>)?; 0, 1, 2, 3, 4, 5);
            crate::content::macros::op_impl!($t$(<$($l),+>)?, $e, 6, |stack: &'b Stack<'a>| {
                let shift = stack.len().saturating_sub(6);
            Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
            stack.get(2 + shift)?, stack.get(3 + shift)?,
            stack.get(4 + shift)?, stack.get(5 + shift)?))
            });
        }
    }

    pub(crate) use display_op;
    pub(crate) use display_variadic_op;
    pub(crate) use op_all;
    pub(crate) use op_impl;
    pub(crate) use op0;
    pub(crate) use op1;
    pub(crate) use op2;
    pub(crate) use op3;
    pub(crate) use op4;
    pub(crate) use op6;
}

#[cfg(test)]
mod tests {
    use super::{TypedIter, UntypedIter};
    use crate::content::ops::TypedInstruction;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn read(content: &[u8]) -> (Vec<u8>, Vec<String>) {
        let mut typed = TypedIter::new(content);
        let data = match typed.next() {
            Some(TypedInstruction::InlineImage(image)) => image.0.raw_data().to_vec(),
            other => panic!("unexpected instruction: {other:?}"),
        };

        let mut untyped = UntypedIter::new(content);
        untyped.next();
        let mut operators = Vec::new();
        while let Some(instruction) = untyped.next() {
            operators.push(String::from_utf8_lossy(instruction.operator.as_ref()).into_owned());
        }

        (data, operators)
    }

    #[test]
    fn inline_image_ends_at_binary_rejection() {
        let (data, operators) = read(b"BI /W 1 /H 1 ID EI EI EI \x80 EI Q");
        assert_eq!(data, b"EI EI EI \x80 ");
        assert_eq!(operators, ["Q"]);
    }

    #[test]
    fn inline_image_ends_where_content_follows() {
        let (data, operators) = read(b"BI /W 1 /H 1 ID x EI ] EI Q");
        assert_eq!(data, b"x EI ] ");
        assert_eq!(operators, ["Q"]);

        let (data, operators) = read(b"BI /W 1 /H 1 ID x EI (\x80) Tj EI Q");
        assert_eq!(data, b"x ");
        assert_eq!(operators, ["Tj", "EI", "Q"]);
    }

    #[test]
    fn inline_image_ends_before_next_inline_image() {
        let (data, operators) = read(b"BI /W 1 /H 1 ID x EI \x80 BI /W 1 ID y EI Q");
        assert_eq!(data, b"x ");
        assert_eq!(operators, ["\u{fffd}", "BI", "Q"]);
    }

    #[test]
    fn inline_image_candidate_within_earlier_string() {
        // From the first `EI`, the second one lies within a string, so the
        // binary byte behind it is no longer within one.
        let (data, operators) = read(b"BI /W 1 /H 1 ID x EI ( EI \x80 (a) Tj EI Q");
        assert_eq!(data, b"x EI ( EI \x80 (a) Tj ");
        assert_eq!(operators, ["Q"]);
    }

    #[test]
    fn inline_image_many_rejected_candidates() {
        let mut content = b"BI /W 1 /H 1 ID ".to_vec();
        for _ in 0..20_000 {
            content.extend_from_slice(b"EI ");
        }
        content.extend_from_slice(b"\x80 EI Q");

        let (data, operators) = read(&content);
        assert_eq!(
            data.len(),
            content.len() - b"BI /W 1 /H 1 ID ".len() - b"EI Q".len()
        );
        assert_eq!(operators, ["Q"]);
    }
}
