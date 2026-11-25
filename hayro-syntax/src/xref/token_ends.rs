//! Where the literal strings and comments of a file end.

use crate::reader::Ends;
use crate::trivia::is_eol_character;
use alloc::vec::Vec;
use core::cell::{Cell, OnceCell};

pub(crate) struct LazyEnds<'a> {
    data: &'a [u8],
    unread: Cell<usize>,
    table: OnceCell<TokenEnds<'a>>,
}

const SCAN_PASSES_BEFORE_INDEX: usize = 4;

impl<'a> LazyEnds<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            unread: Cell::new(data.len().saturating_mul(SCAN_PASSES_BEFORE_INDEX)),
            table: OnceCell::new(),
        }
    }
}

impl Ends for LazyEnds<'_> {
    fn string_end(&self, start: usize) -> Option<Option<usize>> {
        Some(self.table.get()?.string_end(start))
    }

    fn comment_end(&self, start: usize) -> Option<usize> {
        Some(self.table.get()?.comment_end(start))
    }

    fn read(&self, len: usize) {
        let unread = self.unread.get().saturating_sub(len);
        self.unread.set(unread);

        if unread == 0 {
            self.table.get_or_init(|| TokenEnds::new(self.data));
        }
    }
}

#[derive(Debug)]
struct TokenEnds<'a, const BLOCK_LEN: usize = 512> {
    data: &'a [u8],
    blocks: Vec<Block>,
}

#[derive(Debug)]
struct Block {
    depth: isize,
    escaped: bool,
    min: isize,
    next_lower: usize,
    eol: usize,
}

impl<'a, const BLOCK_LEN: usize> TokenEnds<'a, BLOCK_LEN> {
    fn new(data: &'a [u8]) -> Self {
        let mut blocks = Vec::with_capacity(data.len().div_ceil(BLOCK_LEN));
        let mut depth = 0;
        let mut escaped = false;

        for chunk in data.chunks(BLOCK_LEN) {
            let mut block = Block {
                depth,
                escaped,
                min: depth,
                next_lower: 0,
                eol: 0,
            };

            let mut i = usize::from(escaped);
            escaped = false;

            while let Some(&byte) = chunk.get(i) {
                match byte {
                    b'\\' => {
                        i += 1;
                        escaped = i == chunk.len();
                    }
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        block.min = block.min.min(depth);
                    }
                    _ => {}
                }

                i += 1;
            }

            blocks.push(block);
        }

        let count = blocks.len();
        let mut lower = Vec::new();
        let mut eol = data.len();

        for (i, block) in blocks.iter_mut().enumerate().rev() {
            while lower.last().is_some_and(|&(_, min)| min >= block.min) {
                lower.pop();
            }

            let start = i * BLOCK_LEN;
            block.next_lower = lower.last().map_or(count, |&(j, _)| j);
            lower.push((i, block.min));

            if let Some(pos) = data[start..data.len().min(start + BLOCK_LEN)]
                .iter()
                .position(|b| is_eol_character(*b))
            {
                eol = start + pos;
            }

            block.eol = eol;
        }

        Self { data, blocks }
    }

    fn string_end(&self, start: usize) -> Option<usize> {
        let block_idx = start / BLOCK_LEN;
        let block_end = self.data.len().min((block_idx + 1) * BLOCK_LEN);

        let mut open = 1;
        let mut escaped = false;

        for (pos, &byte) in self.data[start + 1..block_end].iter().enumerate() {
            step(byte, &mut open, &mut escaped);

            if open == 0 {
                return Some(start + 1 + pos + 1);
            }
        }

        // After the opening parenthesis, escape parity agrees with the file-wide scan.
        let next = self.blocks.get(block_idx + 1)?;
        debug_assert_eq!(escaped, next.escaped);
        let target = next.depth - (open - 1);

        // Intermediate blocks cannot drop below this block's minimum.
        let mut idx = block_idx + 1;

        while self.blocks.get(idx).is_some_and(|b| b.min >= target) {
            idx = self.blocks[idx].next_lower;
        }

        let block = self.blocks.get(idx)?;
        let block_start = idx * BLOCK_LEN;
        let block_end = self.data.len().min(block_start + BLOCK_LEN);

        self.drop_below(block_start, block_end, block.depth, block.escaped, target)
    }

    fn comment_end(&self, start: usize) -> usize {
        let block_idx = start / BLOCK_LEN;
        let block_end = self.data.len().min((block_idx + 1) * BLOCK_LEN);

        self.data[start + 1..block_end]
            .iter()
            .position(|b| is_eol_character(*b))
            .map(|pos| start + 1 + pos)
            .unwrap_or_else(|| {
                self.blocks
                    .get(block_idx + 1)
                    .map_or(self.data.len(), |b| b.eol)
            })
    }

    fn drop_below(
        &self,
        from: usize,
        to: usize,
        mut depth: isize,
        mut escaped: bool,
        target: isize,
    ) -> Option<usize> {
        for (pos, &byte) in self.data[from..to].iter().enumerate() {
            step(byte, &mut depth, &mut escaped);

            if depth < target {
                return Some(from + pos + 1);
            }
        }

        None
    }
}

#[inline]
fn step(byte: u8, depth: &mut isize, escaped: &mut bool) {
    if *escaped {
        *escaped = false;
    } else {
        match byte {
            b'\\' => *escaped = true,
            b'(' => *depth += 1,
            b')' => *depth -= 1,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LazyEnds, TokenEnds};
    use crate::object::dict::probe_dict;
    use crate::object::{Object, ObjectIdentifier};
    use crate::reader::{Ends, ReaderExt};
    use crate::reader::{Reader, ReaderContext, Unknown, skip_white_spaces_and_comments_in};
    use crate::trivia::Comment;
    use crate::xref::fallback_xref_map_inner;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn looked_up(data: &[u8]) -> LazyEnds<'_> {
        let ends = LazyEnds::new(data);
        ends.read(usize::MAX);
        ends
    }

    fn check_all(alphabet: &[u8], max_len: usize) {
        let mut files = vec![Vec::new()];

        for _ in 0..max_len {
            files = files
                .iter()
                .flat_map(|file| {
                    alphabet.iter().map(move |&b| {
                        let mut file = file.clone();
                        file.push(b);
                        file
                    })
                })
                .collect();

            for file in &files {
                check::<1>(file);
                check::<2>(file);
                check::<3>(file);
                check::<4>(file);
            }
        }
    }

    fn check<const BLOCK_LEN: usize>(data: &[u8]) {
        let ends = TokenEnds::<BLOCK_LEN>::new(data);

        for start in 0..data.len() {
            if data[start] == b'(' {
                let mut r = Reader::new_with(data, start);
                let expected = r
                    .skip::<crate::object::String<'_>>(false)
                    .map(|_| r.offset());
                assert_eq!(ends.string_end(start), expected, "{data:?} at {start}");
            } else if data[start] == b'%' {
                let mut r = Reader::new_with(data, start);
                r.skip::<Comment<'_>>(false).unwrap();
                assert_eq!(ends.comment_end(start), r.offset(), "{data:?} at {start}");
            }
        }
    }

    #[test]
    fn ends_match_skipping() {
        check_all(b"()\\a", 7);
        check_all(b"(%\n\r", 7);
    }

    #[test]
    fn deep_strings() {
        let mut data = b"(".repeat(3000);
        data.extend(b"\\)".repeat(10));
        data.extend(b")".repeat(2000));
        data.extend(b"x\\(".repeat(10));

        check::<1>(&data);
        check::<3>(&data);
        check::<64>(&data);
        check::<1024>(&data);
    }

    struct Random(u64);

    impl Random {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    fn file(random: &mut Random, len: usize) -> Vec<u8> {
        const PIECES: &[&str] = &[
            "(", ")", "\\(", "\\)", "\\", "\\\\", "[", "]", "<<", ">>", "<", ">", "%", "\n", "\r",
            " ", " ", " ", "/A", "/Root", "/Type", "/R#6Fot", "/B#zz", "/K#41", "1", "0", "R",
            "5 0 obj ", "6 0 obj", "obj", "null", "true", "fals", "1.5", "-", "<ab>", "<a b",
            "endobj", "{", "}", "x", "1 0 R", "12 0 R ", "0 R", "#", ".",
        ];
        let mut out = String::new();
        for _ in 0..len {
            match random.below(40) {
                0 => out.push_str(&"[ ".repeat(random.below(8))),
                1 => out.push_str(&"<< /A ".repeat(random.below(8))),
                _ => out.push_str(PIECES[random.below(PIECES.len())]),
            }
        }
        out.into_bytes()
    }

    #[test]
    fn reading_with_ends_reads_as_without() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        let ctx = ReaderContext::dummy();

        for case in 0..160 {
            let len = 1 + random.below(120);
            let data = file(&mut random, len);
            let ends = looked_up(&data);
            let text = String::from_utf8_lossy(&data);

            for at in 0..data.len() {
                let mut plain = Reader::new_with(&data, at);
                let mut looked_up = plain.clone();
                let expected = plain.read::<ObjectIdentifier>(&ctx).map(|_| plain.offset());
                let found =
                    ObjectIdentifier::read_in(&mut looked_up, &ends).map(|_| looked_up.offset());
                assert_eq!(found, expected, "case {case}, header at {at} of {text:?}");

                let mut plain = Reader::new_with(&data, at);
                let mut looked_up = plain.clone();
                plain.skip_white_spaces_and_comments();
                skip_white_spaces_and_comments_in(&mut looked_up, &ends);
                assert_eq!(looked_up.offset(), plain.offset(), "case {case} at {at}");
                let expected = plain.skip::<Object<'_>>(false).map(|_| plain.offset());
                let found =
                    Object::skip_in(&mut looked_up, false, 0, &ends).map(|_| looked_up.offset());
                assert_eq!(found, expected, "case {case}, object at {at} of {text:?}");

                let mut plain = Reader::new_with(&data, at);
                let mut looked_up = plain.clone();
                let expected = probe_dict(&mut plain, &ctx, &Unknown, Some(b"<<"), b">>")
                    .map(|p| (plain.offset(), p.data, p.has_root, p.has_type));
                let found = probe_dict(&mut looked_up, &ctx, &ends, Some(b"<<"), b">>")
                    .map(|p| (looked_up.offset(), p.data, p.has_root, p.has_type));
                assert_eq!(
                    found, expected,
                    "case {case}, dictionary at {at} of {text:?}"
                );
            }
        }
    }

    #[test]
    fn reading_nested_objects_with_ends_reads_as_without() {
        let max = crate::object::MAX_OBJECT_NESTING_DEPTH;

        for depth in [max - 1, max, max + 1] {
            for (open, close) in [("[ (a) ", "] "), ("<< /A (a) /B ", ">> ")] {
                for tail in ["", "(", "% x\n", ")"] {
                    let text = format!("{}{tail}{}", open.repeat(depth), close.repeat(depth));
                    let data = text.as_bytes();
                    let ends = looked_up(data);

                    for at in [0, open.len(), data.len() / 2] {
                        let mut plain = Reader::new_with(data, at);
                        let mut looked_up = plain.clone();
                        let expected = plain.skip::<Object<'_>>(false).map(|_| plain.offset());
                        let found = Object::skip_in(&mut looked_up, false, 0, &ends)
                            .map(|_| looked_up.offset());
                        assert_eq!(found, expected, "{depth} deep, {tail:?} inside, at {at}");
                    }
                }
            }
        }
    }

    #[test]
    fn index_waits_for_repeated_scans() {
        let value = "x".repeat(128);
        let text: String = (1..=20)
            .map(|i| format!("{i} 0 obj << /A ({value}) >> endobj\n"))
            .collect();
        let data = crate::PdfData::from(text.into_bytes());
        let ends = LazyEnds::new(data.as_ref());
        let rebuild =
            || fallback_xref_map_inner(&data, &ends, ReaderContext::dummy(), false, b"").0;
        let expected = rebuild();
        assert_eq!(expected.len(), 20);
        assert_eq!(rebuild(), expected);
        assert!(ends.table.get().is_none());
        for _ in 0..2 {
            assert_eq!(rebuild(), expected);
        }
        assert!(ends.table.get().is_some());
    }
}
