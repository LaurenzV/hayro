use crate::object::dict::keys::{ROOT, TYPE};
use crate::object::name::decoded_bytes;
use crate::object::{MAX_OBJECT_NESTING_DEPTH, MaybeRef, Name, Object};
use crate::reader::{Reader, ReaderExt};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

const MAX_FAILED_CACHE: usize = 4 * MAX_OBJECT_NESTING_DEPTH;
const MAX_BATCH_BYTES: usize = 12 * 1024;
const MAX_BATCH_STEPS: usize = 8 * MAX_OBJECT_NESTING_DEPTH;

#[derive(Default)]
pub(super) struct Containers {
    failed: BTreeSet<usize>,
    last_ok: Option<(usize, Container)>,
    frames: Vec<Frame>,
}

#[derive(Clone, Copy)]
pub(super) struct Container {
    pub(super) end: usize,
    pub(super) has_root: bool,
    pub(super) has_type: bool,
    height: u16,
}

struct Frame {
    start: usize,
    dict: bool,
    value: Container,
}

impl Frame {
    fn open(r: &mut Reader<'_>) -> Option<Self> {
        let start = r.offset();
        let dict = if r.forward_tag(b"[").is_some() {
            false
        } else {
            r.forward_tag(b"<<")?;
            true
        };
        Some(Self {
            start,
            dict,
            value: Container {
                end: 0,
                has_root: false,
                has_type: false,
                height: 1,
            },
        })
    }

    fn read_dict_key(&mut self, r: &mut Reader<'_>) -> bool {
        let Some(raw) = r.skip::<Name<'_>>(false) else {
            return false;
        };
        let mut prefix = [0; 5];
        let mut len = 0;
        for (slot, byte) in prefix.iter_mut().zip(decoded_bytes(&raw[1..])) {
            *slot = byte;
            len += 1;
        }
        self.value.has_root |= &prefix[..len] == ROOT;
        self.value.has_type |= &prefix[..len] == TYPE;
        true
    }
}

impl Containers {
    pub(super) fn skip_object(&mut self, r: &mut Reader<'_>) -> Option<()> {
        if r.peek_byte() == Some(b'[') || r.peek_bytes(2) == Some(b"<<") {
            self.resolve(r.clone()).map(|value| r.jump(value.end))
        } else {
            r.skip::<Object<'_>>(false).map(|_| ())
        }
    }

    pub(super) fn evict_before(&mut self, pos: usize) {
        while self.failed.first().is_some_and(|&offset| offset < pos) {
            self.failed.pop_first();
        }
    }

    pub(super) fn resolve(&mut self, mut r: Reader<'_>) -> Option<Container> {
        let start = r.offset();
        if self.failed.contains(&start) {
            return None;
        }
        if let Some((offset, value)) = self.last_ok
            && offset == start
        {
            return Some(value);
        }

        self.frames.push(Frame::open(&mut r)?);
        let mut doomed_prefix = 0;
        let mut batch_steps = 0;
        let mut batch_truncated = false;
        while let Some(frame) = self.frames.last_mut() {
            if doomed_prefix > 0 {
                if doomed_prefix >= MAX_OBJECT_NESTING_DEPTH || batch_steps == MAX_BATCH_STEPS {
                    break;
                }
                if batch_steps == 0 {
                    let limit = r.offset().saturating_add(MAX_BATCH_BYTES);
                    if limit < r.data.len() {
                        r = Reader::new_with(&r.data[..limit], r.offset());
                        batch_truncated = true;
                    }
                }
                batch_steps += 1;
            }
            r.skip_white_spaces_and_comments();
            let closer: &[u8] = if frame.dict { b">>" } else { b"]" };
            if r.forward_tag(closer).is_some() {
                let mut closed = self.frames.pop().unwrap().value;
                closed.end = r.offset();
                let Some(parent) = self.frames.last_mut() else {
                    self.last_ok = Some((start, closed));
                    return Some(closed);
                };
                parent.value.height = parent.value.height.max(closed.height + 1);
                if usize::from(parent.value.height) > MAX_OBJECT_NESTING_DEPTH {
                    doomed_prefix = self.frames.len();
                    break;
                }
                continue;
            }

            let has_key = frame.dict && frame.read_dict_key(&mut r);
            if has_key {
                r.skip_white_spaces_and_comments();
            }
            if let Some(open) = Frame::open(&mut r) {
                if self.failed.contains(&open.start) {
                    doomed_prefix = self.frames.len();
                    break;
                }
                self.frames.push(open);
                doomed_prefix =
                    doomed_prefix.max(self.frames.len().saturating_sub(MAX_OBJECT_NESTING_DEPTH));
                continue;
            }
            let scalar = if !frame.dict || has_key {
                r.skip::<MaybeRef<Object<'_>>>(false)
            } else {
                r.skip::<Object<'_>>(false)
            };
            if scalar.is_none() {
                if !batch_truncated {
                    doomed_prefix = self.frames.len();
                }
                break;
            }
        }
        // Only the outer prefix is known to fail when the batch budget runs out.
        for frame in self.frames.drain(..doomed_prefix) {
            if self.failed.len() == MAX_FAILED_CACHE {
                if self.failed.last().is_some_and(|&last| last < frame.start) {
                    break;
                }
                self.failed.pop_last();
            }
            self.failed.insert(frame.start);
        }
        self.frames.clear();
        None
    }
}

#[cfg(test)]
mod tests;
