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
use crate::object::Stream;
use crate::object::dict::InlineImageDict;
use crate::object::name::{Name, skip_name_like};
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use log::warn;
use smallvec::SmallVec;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;

// 6 operands are used for example for ctm or cubic curves,
// but anything above should be pretty rare (only for example for
// DeviceN color spaces)
const OPERANDS_THRESHOLD: usize = 6;

impl Debug for Operator<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
    fn read(r: &mut Reader<'a>, _: &ReaderContext) -> Option<Self> {
        let data = {
            let start = r.offset();
            skip_name_like(r, false)?;
            let end = r.offset();
            let data = r.range(start..end).unwrap();

            if data.is_empty() {
                return None;
            }

            data
        };

        Some(Operator(Name::from_unescaped(data)))
    }
}

/// An iterator over operators in the PDF content streams, providing raw access to the instructions.
#[derive(Clone)]
pub struct UntypedIter<'a> {
    reader: Reader<'a>,
    stack: Stack<'a>,
}

impl<'a> UntypedIter<'a> {
    /// Create a new untyped iterator.
    pub fn new(data: &'a [u8]) -> UntypedIter<'a> {
        Self {
            reader: Reader::new(data),
            stack: Stack::new(),
        }
    }

    /// Create a new empty untyped iterator.
    pub fn empty() -> UntypedIter<'a> {
        Self {
            reader: Reader::new(&[]),
            stack: Stack::new(),
        }
    }
}

impl<'a> Iterator for UntypedIter<'a> {
    type Item = Instruction<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.stack.clear();

        self.reader.skip_white_spaces_and_comments();

        while !self.reader.at_end() {
            // I believe booleans/null never appear as an operator?
            if matches!(
                self.reader.peek_byte()?,
                b'/' | b'.' | b'+' | b'-' | b'0'..=b'9' | b'[' | b'<' | b'('
            ) {
                self.stack
                    .push(self.reader.read_without_context::<Object>()?);
            } else {
                let operator = match self.reader.read_without_context::<Operator>() {
                    Some(o) => o,
                    None => {
                        warn!("failed to read operator in content stream");

                        self.reader.jump_to_end();
                        return None;
                    }
                };

                // Inline images need special casing...
                if operator.as_ref() == b"BI" {
                    // The ID operator will already be consumed by this.
                    let inline_dict = self.reader.read_without_context::<InlineImageDict>()?;
                    let dict = inline_dict.get_dict().clone();

                    // One whitespace after "ID".
                    self.reader.read_white_space()?;

                    let stream_data = self.reader.tail()?;
                    let start_offset = self.reader.offset();

                    'outer: while let Some(bytes) = self.reader.peek_bytes(2) {
                        if bytes == b"EI" {
                            let end_offset = self.reader.offset() - start_offset;
                            let image_data = &stream_data[..end_offset];

                            let stream = Stream::new(image_data, dict.clone());

                            // Note that there is a possibility that the encoded stream data
                            // contains the "EI" operator as part of the data, in which case we
                            // cannot confidently know whether we have hit the actual end of the
                            // stream. See also <https://github.com/pdf-association/pdf-issues/issues/543>
                            // PDF 2.0 does have a `/Length` attribute we can read, but since it's relatively
                            // new we don't bother trying to read it.
                            let tail = &self.reader.tail()?[2..];
                            let mut find_reader = Reader::new(tail);

                            while let Some(bytes) = find_reader.peek_bytes(2) {
                                if bytes == b"EI" {
                                    let analyze_data = &tail;

                                    // If there is any binary data in-between, we for sure
                                    // have not reached the end.
                                    if analyze_data.iter().any(|c| !c.is_ascii()) {
                                        self.reader.read_bytes(2)?;
                                        continue 'outer;
                                    }

                                    // Otherwise, the only possibility that we reached an
                                    // "EI", even though the previous one was valid, is
                                    // that it's part of a string in the content
                                    // stream that follows the inline image. Therefore,
                                    // it should be valid to interpret `tail` as a content
                                    // stream and there should be at least one text-related
                                    // operator that can be parsed correctly.

                                    let iter = TypedIter::new(tail);
                                    let mut found = false;

                                    for (counter, op) in iter.enumerate() {
                                        // If we have read more than 20 valid operators, it should be
                                        // safe to assume that we are in a content stream, so abort
                                        // early. The only situation where this could reasonably
                                        // be violated is if we have 20 subsequent instances of
                                        // q/Q in the image data, which seems very unlikely.
                                        if counter >= 20 {
                                            found = true;
                                            break;
                                        }

                                        if matches!(
                                            op,
                                            TypedInstruction::NextLineAndShowText(_)
                                                | TypedInstruction::ShowText(_)
                                                | TypedInstruction::ShowTexts(_)
                                                | TypedInstruction::ShowTextWithParameters(_)
                                        ) {
                                            // Now it should be safe to assume that the
                                            // previous `EI` was the correct one.
                                            found = true;
                                            break;
                                        }
                                    }

                                    if !found {
                                        // Seems like the data in-between is not a valid content
                                        // stream, so we are likely still within the image data.
                                        self.reader.read_bytes(2)?;
                                        continue 'outer;
                                    }
                                } else if bytes == b"BI" {
                                    // Possibly another inline image, if so, the previously found "EI"
                                    // is indeed the end of data.
                                    let mut cloned = find_reader.clone();
                                    cloned.read_bytes(2)?;
                                    if cloned.read_without_context::<InlineImageDict>().is_some() {
                                        break;
                                    }
                                }

                                find_reader.read_byte()?;
                            }

                            self.stack.push(Object::Stream(stream));

                            self.reader.read_bytes(2)?;
                            self.reader.skip_white_spaces();

                            break;
                        } else {
                            self.reader.read_byte()?;
                        }
                    }
                }

                return Some(Instruction {
                    operands: self.stack.clone(),
                    operator,
                });
            }

            self.reader.skip_white_spaces_and_comments();
        }

        None
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
    pub fn new(data: &'a [u8]) -> TypedIter<'a> {
        Self {
            untyped: UntypedIter::new(data),
        }
    }

    pub(crate) fn from_untyped(untyped: UntypedIter<'a>) -> TypedIter<'a> {
        Self { untyped }
    }
}

impl<'a> Iterator for TypedIter<'a> {
    type Item = TypedInstruction<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let op = &self.untyped.next()?;
        match TypedInstruction::dispatch(op) {
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
                    Some(TypedInstruction::Fallback(op.operator.clone()))
                }
            }
        }
    }
}

/// An instruction (= operator and its operands) in a content stream.
pub struct Instruction<'a> {
    /// The stack containing the operands.
    pub operands: Stack<'a>,
    /// The actual operator.
    pub operator: Operator<'a>,
}

impl<'a> Instruction<'a> {
    /// An iterator over the operands of the instruction.
    pub fn operands(self) -> OperandIterator<'a> {
        OperandIterator::new(self.operands)
    }
}

/// A stack holding the arguments of an operator.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack<'a>(SmallVec<[Object<'a>; OPERANDS_THRESHOLD]>);

impl<'a> Default for Stack<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Stack<'a> {
    /// Create a new, empty stack.
    pub fn new() -> Self {
        Self(SmallVec::new())
    }

    fn push(&mut self, operand: Object<'a>) {
        self.0.push(operand);
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get<T>(&self, index: usize) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        self.0.get(index).and_then(|e| e.clone().cast::<T>())
    }

    fn get_all<T>(&self) -> Option<SmallVec<[T; OPERANDS_THRESHOLD]>>
    where
        T: ObjectLike<'a>,
    {
        let mut operands = SmallVec::new();

        for op in &self.0 {
            let converted = op.clone().cast::<T>()?;
            operands.push(converted);
        }

        Some(operands)
    }
}

/// An iterator over the operands of an operator.
pub struct OperandIterator<'a> {
    stack: Stack<'a>,
    cur_index: usize,
}

impl<'a> OperandIterator<'a> {
    fn new(stack: Stack<'a>) -> Self {
        Self {
            stack,
            cur_index: 0,
        }
    }
}

impl<'a> Iterator for OperandIterator<'a> {
    type Item = Object<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.stack.get::<Object<'a>>(self.cur_index) {
            self.cur_index += 1;

            Some(item)
        } else {
            None
        }
    }
}

pub(crate) trait OperatorTrait<'a>
where
    Self: Sized + Into<TypedInstruction<'a>> + TryFrom<TypedInstruction<'a>>,
{
    const OPERATOR: &'static str;

    fn from_stack(stack: &Stack<'a>) -> Option<Self>;
}

mod macros {
    macro_rules! op_impl {
        ($t:ident $(<$l:lifetime>),*, $e:expr, $n:expr, $body:expr) => {
            impl<'a> OperatorTrait<'a> for $t$(<$l>),* {
                const OPERATOR: &'static str = $e;

                fn from_stack(stack: &Stack<'a>) -> Option<Self> {
                    if $n != u8::MAX as usize {
                        if stack.len() != $n {
                            warn!("wrong stack length {} for operator {}, expected {}", stack.len(), Self::OPERATOR, $n);
                        }
                    }

                    $body(stack).or_else(|| {
                        warn!("failed to convert operands for operator {}", Self::OPERATOR);

                        None
                    })
                }
            }

            impl<'a> From<$t$(<$l>),*> for TypedInstruction<'a> {
                fn from(value: $t$(<$l>),*) -> Self {
                    TypedInstruction::$t(value)
                }
            }

            impl<'a> TryFrom<TypedInstruction<'a>> for $t$(<$l>),* {
                type Error = ();

                fn try_from(value: TypedInstruction<'a>) -> std::result::Result<Self, Self::Error> {
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
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 0, |_| Some(Self));
        }
    }

    macro_rules! op1 {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 1, |stack: &Stack<'a>| {
                let shift = stack.len().saturating_sub(1);
                Some(Self(stack.get(0 + shift)?))
            });
        }
    }

    macro_rules! op_all {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, u8::MAX as usize, |stack: &Stack<'a>|
            Some(Self(stack.get_all()?)));
        }
    }

    macro_rules! op2 {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 2, |stack: &Stack<'a>| {
                let shift = stack.len().saturating_sub(2);
                Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?))
            });
        }
    }

    macro_rules! op3 {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 3, |stack: &Stack<'a>| {
                let shift = stack.len().saturating_sub(3);
                Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
                stack.get(2 + shift)?))
            });
        }
    }

    macro_rules! op4 {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 4, |stack: &Stack<'a>| {
               let shift = stack.len().saturating_sub(4);
            Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
            stack.get(2 + shift)?, stack.get(3 + shift)?))
            });
        }
    }

    macro_rules! op6 {
        ($t:ident $(<$l:lifetime>),*, $e:expr) => {
            crate::content::macros::op_impl!($t$(<$l>),*, $e, 6, |stack: &Stack<'a>| {
                let shift = stack.len().saturating_sub(6);
            Some(Self(stack.get(0 + shift)?, stack.get(1 + shift)?,
            stack.get(2 + shift)?, stack.get(3 + shift)?,
            stack.get(4 + shift)?, stack.get(5 + shift)?))
            });
        }
    }

    pub(crate) use op_all;
    pub(crate) use op_impl;
    pub(crate) use op0;
    pub(crate) use op1;
    pub(crate) use op2;
    pub(crate) use op3;
    pub(crate) use op4;
    pub(crate) use op6;
}
