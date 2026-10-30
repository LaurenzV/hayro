//! Where the literal strings and comments of a file end.

use crate::reader::Ends;
use crate::trivia::is_eol_character;
use alloc::vec::Vec;
use core::cell::{Cell, OnceCell};

/// The ends of strings and comments for rebuilding the xref table, which reads objects
/// from every `<<` and every object header it finds, also from those inside a string or
/// comment that an earlier read went through already. Reading that string or comment
/// again from each start would read the rest of it once per start, quadratic in the size
/// of the file.
///
/// Strings and comments are read until they have taken as many bytes as the file has.
/// Then one pass over the file finds where any of them ends, which costs about as much
/// as the reads before it, so the work stays linear, and a file whose strings and
/// comments are read a few times each isn't passed over at all.
pub(crate) struct LazyEnds<'a> {
    data: &'a [u8],
    unread: Cell<usize>,
    table: OnceCell<TokenEnds<'a>>,
}

impl<'a> LazyEnds<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            unread: Cell::new(data.len()),
            table: OnceCell::new(),
        }
    }

    /// Ends that are looked up from the start, in blocks of `block_len` bytes.
    #[cfg(test)]
    fn looked_up(data: &'a [u8], block_len: usize) -> Self {
        Self {
            data,
            unread: Cell::new(0),
            table: OnceCell::from(TokenEnds::with_block_len(data, block_len)),
        }
    }

    fn table(&self, data: &[u8]) -> Option<&TokenEnds<'a>> {
        core::ptr::eq(data, self.data)
            .then(|| self.table.get())
            .flatten()
    }
}

impl Ends for LazyEnds<'_> {
    fn string_end(&self, data: &[u8], start: usize) -> Option<Option<usize>> {
        Some(self.table(data)?.string_end(start))
    }

    fn comment_end(&self, data: &[u8], start: usize) -> Option<usize> {
        Some(self.table(data)?.comment_end(start))
    }

    fn read(&self, len: usize) {
        work(len);
        let unread = self.unread.get().saturating_sub(len);
        self.unread.set(unread);

        if unread == 0 {
            self.table.get_or_init(|| TokenEnds::new(self.data));
        }
    }
}

#[cfg(test)]
std::thread_local! {
    /// The bytes read on this thread to find where strings and comments end while
    /// rebuilding xref tables, which the tests bound.
    pub(crate) static WORK: Cell<usize> = const { Cell::new(0) };
}

#[inline]
fn work(_bytes: usize) {
    #[cfg(test)]
    WORK.set(WORK.get() + _bytes);
}

/// Where the literal strings and comments of a file end, found by one pass over it.
///
/// The file is split into blocks, and a lookup reads at most a few blocks of it.
#[derive(Debug)]
pub(crate) struct TokenEnds<'a> {
    data: &'a [u8],
    block_len: usize,
    blocks: Vec<Block>,
}

#[derive(Debug)]
struct Block {
    /// The depth before the block's first byte: the number of unescaped `(` minus the
    /// number of unescaped `)` from the start of the file.
    depth: isize,
    /// Whether a backslash escapes the block's first byte.
    escaped: bool,
    /// The least depth before the block or after one of its bytes.
    min: isize,
    /// The first later block whose `min` is less than this one's, or the number of blocks.
    next_lower: usize,
    /// The first end-of-line byte at or after the block's start, or the length of the file.
    eol: usize,
}

const BLOCK_LEN: usize = 512;

impl<'a> TokenEnds<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self::with_block_len(data, BLOCK_LEN)
    }

    fn with_block_len(data: &'a [u8], block_len: usize) -> Self {
        work(data.len());
        let mut blocks = Vec::with_capacity(data.len().div_ceil(block_len));
        let mut depth = 0;
        let mut escaped = false;

        for chunk in data.chunks(block_len) {
            let mut block = Block {
                depth,
                escaped,
                min: depth,
                next_lower: 0,
                eol: 0,
            };

            // Only a `)` lowers the depth, so `min` is the least depth before the block or
            // after one of its `)`.
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

            let start = i * block_len;
            block.next_lower = lower.last().map_or(count, |&(j, _)| j);
            lower.push((i, block.min));

            if let Some(pos) = data[start..data.len().min(start + block_len)]
                .iter()
                .position(|b| is_eol_character(*b))
            {
                eol = start + pos;
            }

            block.eol = eol;
        }

        Self {
            data,
            block_len,
            blocks,
        }
    }

    /// The offset after the `)` that ends the literal string starting with the `(` at
    /// `start`, as reading the string from there finds it, or `None` if the string doesn't
    /// end.
    pub(crate) fn string_end(&self, start: usize) -> Option<usize> {
        let block_idx = start / self.block_len;
        let block_end = self.data.len().min((block_idx + 1) * self.block_len);

        // Up to the end of its block, read the string as skipping it does.
        work(block_end - start);
        let mut open = 1;
        let mut escaped = false;

        for (pos, &byte) in self.data[start + 1..block_end].iter().enumerate() {
            step(byte, &mut open, &mut escaped);

            if open == 0 {
                return Some(start + 1 + pos + 1);
            }
        }

        // Since the `(` at `start` isn't a backslash, whether a backslash escapes a byte
        // after it doesn't depend on where counting started, so the string ends after the
        // first `)` where the depth counted from the start of the file drops below the
        // depth after `start`.
        let next = self.blocks.get(block_idx + 1)?;
        debug_assert_eq!(escaped, next.escaped);
        let target = next.depth - (open - 1);

        // The blocks between a block and its `next_lower` don't get lower than it does,
        // so they don't get below `target` either.
        let mut idx = block_idx + 1;

        while self.blocks.get(idx).is_some_and(|b| b.min >= target) {
            work(1);
            idx = self.blocks[idx].next_lower;
        }

        let block = self.blocks.get(idx)?;
        let block_start = idx * self.block_len;
        let block_end = self.data.len().min(block_start + self.block_len);
        work(block_end - block_start);

        self.drop_below(block_start, block_end, block.depth, block.escaped, target)
    }

    /// The offset of the first end-of-line byte after the `%` at `start`, or the length
    /// of the file if there is none.
    pub(crate) fn comment_end(&self, start: usize) -> usize {
        let block_idx = start / self.block_len;
        let block_end = self.data.len().min((block_idx + 1) * self.block_len);
        work(block_end - start);

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

    /// The offset after the first byte in `from..to` after which the depth is less than
    /// `target`.
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

/// Count `byte` the way skipping a literal string does.
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
    use crate::reader::ReaderExt;
    use crate::reader::{Reader, ReaderContext, Unknown, skip_white_spaces_and_comments_in};
    use crate::trivia::Comment;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Every string and comment end in every file of up to `max_len` bytes made of
    /// `alphabet`, compared with skipping without the lookup.
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
                for block_len in 1..=4 {
                    check(file, block_len);
                }
            }
        }
    }

    fn check(data: &[u8], block_len: usize) {
        let ends = TokenEnds::with_block_len(data, block_len);

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
        check_all(b"()\\a", 9);
        check_all(b"(%\n\r", 7);
    }

    #[test]
    fn deep_strings() {
        let mut data = b"(".repeat(3000);
        data.extend(b"\\)".repeat(10));
        data.extend(b")".repeat(2000));
        data.extend(b"x\\(".repeat(10));

        for block_len in [1, 3, 64, 1024] {
            check(&data, block_len);
        }
    }

    /// A generator of pseudo-random numbers, the same in every run.
    struct Random(u64);

    impl Random {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    /// A file of `len` pieces of tokens, strings, escapes, comments and headers that
    /// open, close and nest in every way.
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

    // Reading with the ends looked up finds what reading through every string and
    // comment finds, at every offset of random files.
    #[test]
    fn reading_with_ends_reads_as_without() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        let ctx = ReaderContext::dummy();

        for case in 0..2000 {
            let len = 1 + random.below(120);
            let data = file(&mut random, len);
            let ends = LazyEnds::looked_up(&data, 1 + random.below(64));
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

    // So too where arrays and dictionaries nest as deep as reading allows, and deeper.
    #[test]
    fn reading_nested_objects_with_ends_reads_as_without() {
        let max = crate::object::MAX_OBJECT_NESTING_DEPTH;

        for depth in [max - 1, max, max + 1] {
            for (open, close) in [("[ (a) ", "] "), ("<< /A (a) /B ", ">> ")] {
                for tail in ["", "(", "% x\n", ")"] {
                    let text = format!("{}{tail}{}", open.repeat(depth), close.repeat(depth));
                    let data = text.as_bytes();
                    let ends = LazyEnds::looked_up(data, 7);

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

    /// A file whose table is rebuilt, with object 4 holding `body`.
    fn around(body: &str) -> Vec<u8> {
        format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
             2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>\nendobj\n\
             4 0 obj\n{body}\nendobj\n\
             trailer\n<< /Root 1 0 R >>\nstartxref\n999999999\n%%EOF\n"
        )
        .into_bytes()
    }

    // Rebuilding the table reads each part of a file a bounded number of times to find
    // where strings and comments end, however many objects start inside them: each
    // family's work doubles with the file, where reading every string and comment from
    // each start quadrupled it.
    #[test]
    fn rebuilding_finds_ends_in_linear_time() {
        type Family = (&'static str, fn(usize) -> Vec<u8>);
        let families: [Family; 8] = [
            ("dictionaries over unterminated strings", |n| {
                around(&"<< /A ( ".repeat(n))
            }),
            ("dictionaries over strings that end", |n| {
                around(&format!("{}{}", "<< /A ( ".repeat(n), ")".repeat(n)))
            }),
            ("headers over unterminated strings", |n| {
                around(&"5 0 obj ( ".repeat(n))
            }),
            ("dictionaries over a comment", |n| {
                around(&"<< % ".repeat(n))
            }),
            ("numbers over a comment", |n| around(&"1 % ".repeat(n))),
            ("headers over a comment", |n| around(&"1 0 % ".repeat(n))),
            // Each string ends in the line before another `%`, where reading enters a
            // comment to the end of the line.
            ("comments entered at each string end of a line", |n| {
                around(&format!(
                    "{}{}\n",
                    "5 0 obj [ ( ".repeat(n),
                    "% ) ".repeat(n)
                ))
            }),
            // Strings opened at escaped parentheses end with the string around them.
            ("headers in strings that end together", |n| {
                let tail = "/x ".repeat(n);
                around(&format!("[ ( {}) {tail}]", "5 0 obj [ \\( ".repeat(n)))
            }),
        ];

        for (family, file) in families {
            let [small, large] = [1000, 2000].map(|n| {
                let file = file(n);
                super::WORK.set(0);
                let pdf = crate::Pdf::new(file).unwrap();
                assert_eq!(pdf.pages().len(), 1, "{family}");
                super::WORK.get()
            });

            assert!(
                large <= small * 22 / 10,
                "{family}: {small} bytes read for 1000, {large} for 2000"
            );
        }
    }
}
