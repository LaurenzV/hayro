//! Reading objects with a `ParseBudget`.

use hayro_syntax::budget::{BudgetRef, Exhausted, ParseBudget};
use hayro_syntax::object::{Array, Dict, FromBytes, MaybeRef, Name, Object};
use std::sync::Mutex;
use std::time::Instant;

/// A budget with limits, which records what it was charged and keeps
/// refusing once it refused.
#[derive(Default)]
struct Limits {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    work_limit: Option<usize>,
    memory_limit: Option<usize>,
    cancel_after_polls: Option<usize>,
    deadline: Option<Instant>,
    work: usize,
    polls: usize,
    polls_after_refusal: usize,
    memory: usize,
    peak_memory: usize,
    refused: bool,
}

impl Limits {
    fn with(f: impl FnOnce(&mut State)) -> Self {
        let limits = Self::default();
        f(&mut limits.state.lock().unwrap());
        limits
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

impl ParseBudget for Limits {
    fn work(&self, bytes: usize) -> Result<(), Exhausted> {
        let mut s = self.state();
        if s.refused {
            s.polls_after_refusal += 1;
            return Err(Exhausted);
        }
        s.polls += 1;
        let cancelled = s.cancel_after_polls.is_some_and(|n| s.polls > n);
        let late = s.deadline.is_some_and(|d| Instant::now() >= d);
        let over = s.work_limit.is_some_and(|l| s.work + bytes > l);
        if cancelled || late || over {
            s.refused = true;
            return Err(Exhausted);
        }
        s.work += bytes;
        Ok(())
    }

    fn allocate(&self, bytes: usize) -> Result<(), Exhausted> {
        let mut s = self.state();
        if s.refused || s.memory_limit.is_some_and(|l| s.memory + bytes > l) {
            s.refused = true;
            return Err(Exhausted);
        }
        s.memory += bytes;
        s.peak_memory = s.peak_memory.max(s.memory);
        Ok(())
    }

    fn release(&self, bytes: usize) {
        let mut s = self.state();
        s.memory = s
            .memory
            .checked_sub(bytes)
            .expect("released more than charged");
    }

    fn exhausted(&self) -> bool {
        self.state().refused
    }
}

fn large_dict(entries: usize) -> Vec<u8> {
    let mut data = b"<<".to_vec();
    for i in 0..entries {
        data.extend_from_slice(format!("/K{i} {i} ").as_bytes());
    }
    data.extend_from_slice(b">>");
    data
}

const VALID: &[&[u8]] = &[
    b"null",
    b"true",
    b"-12.5",
    b"/Name#20With#20Escapes",
    b"(literal \\(string\\) with \\101 escapes\\\nand a line)",
    b"<48656C6C6F20776F726C64206865782068657820686578>",
    b"[1 2 0 R /A (b) <63> [[]] << /K /V >>]",
    b"<< /Type /Page /Kids [1 0 R 2 0 R] /Nested << /A [1 2 3] /B (x) >> >>",
    b"<< /Length 3 >> stream\nabc\nendstream",
    b"<< /Length 99 >> stream\nwrong length\nendstream",
    // Damaged objects read the same with and without a budget.
    b"<< /A 1 /B >>",
    b"[1 2 (unterminated",
    b"<< /A [1 2 >> >>",
];

#[test]
fn valid_and_damaged_objects_read_the_same() {
    for input in VALID {
        let limits = Limits::default();
        let plain = MaybeRef::<Object<'_>>::from_bytes(input);
        let budget = BudgetRef::new(&limits);
        let budgeted = MaybeRef::<Object<'_>>::from_bytes_with_budget(input, &budget).unwrap();
        // Every scanned byte is charged, and bytes that a read scans again,
        // such as a stream's /Length, are charged again.
        assert!(limits.state().work <= 3 * input.len());
        assert_eq!(
            format!("{plain:?}"),
            format!("{budgeted:?}"),
            "{}",
            String::from_utf8_lossy(input)
        );
        assert!(!limits.state().refused);
    }
}

#[test]
fn lazy_reads_return_the_same_values() {
    let input = b"<< /A << /B [1 (two) /Three] >> >>";
    let limits = Limits::default();
    let budget = BudgetRef::new(&limits);
    let dict = Dict::from_bytes_with_budget(input, &budget)
        .unwrap()
        .unwrap();
    let inner = dict.get::<Dict<'_>>("A").unwrap();
    let array = inner.get::<Array<'_>>("B").unwrap();
    let items = array.iter::<Object<'_>>().collect::<Vec<_>>();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2], Object::Name(Name::new(b"Three").unwrap()));
}

#[test]
fn a_long_name_stops_at_the_work_limit() {
    let mut input = b"/".to_vec();
    input.resize(10 << 20, b'a');
    let limits = Limits::with(|s| s.work_limit = Some(64 << 10));
    assert_eq!(
        Name::from_bytes_with_budget(&input, &BudgetRef::new(&limits)),
        Err(Exhausted)
    );
    let s = limits.state();
    assert!(s.refused);
    assert!(s.work <= 64 << 10);
    // The reader stopped at the refusal instead of scanning the rest.
    assert!(s.polls_after_refusal <= 2, "{}", s.polls_after_refusal);
}

#[test]
fn a_long_string_is_cancelled_while_it_is_read() {
    for (open, close) in [(b'(', b')'), (b'<', b'>')] {
        let mut input = vec![open];
        input.resize(8 << 20, b'4');
        input.push(close);
        // The token is cancelled during the read, after 10 windows.
        let limits = Limits::with(|s| s.cancel_after_polls = Some(10));
        assert_eq!(
            hayro_syntax::object::String::from_bytes_with_budget(&input, &BudgetRef::new(&limits)),
            Err(Exhausted)
        );
        let s = limits.state();
        assert!(s.refused);
        assert_eq!(s.polls, 11);
        assert!(s.work < 64 << 10);
        assert!(s.polls_after_refusal <= 2, "{}", s.polls_after_refusal);
    }
}

#[test]
fn a_deadline_stops_a_large_object() {
    let input = large_dict(200_000);
    let limits = Limits::with(|s| s.deadline = Some(Instant::now()));
    assert!(Dict::from_bytes_with_budget(&input, &BudgetRef::new(&limits)).is_err());
    let s = limits.state();
    assert!(s.refused);
    assert_eq!(s.work, 0);
}

#[test]
fn a_large_dictionary_charges_its_table_before_allocating_it() {
    let input = large_dict(100_000);

    let unlimited = Limits::default();
    let budget = BudgetRef::new(&unlimited);
    let dict = Dict::from_bytes_with_budget(&input, &budget)
        .unwrap()
        .unwrap();
    assert_eq!(dict.len(), 100_000);
    let charged = unlimited.state().memory;
    assert!(charged >= 100_000 * size_of::<(Name<'_>, usize)>());
    drop(dict);
    assert_eq!(
        unlimited.state().memory,
        0,
        "dropping the dictionary releases its table"
    );

    for limit in [0, 1 << 10, 1 << 20] {
        let limits = Limits::with(|s| s.memory_limit = Some(limit));
        assert!(Dict::from_bytes_with_budget(&input, &BudgetRef::new(&limits)).is_err());
        let s = limits.state();
        assert!(s.refused);
        assert!(s.peak_memory <= limit);
        assert_eq!(s.memory, 0, "a refused read releases what it charged");
    }
}

#[test]
fn escaped_names_and_strings_charge_their_buffers() {
    let mut name = b"/#41".to_vec();
    name.resize(1 << 16, b'b');
    let mut literal = b"(\\n".to_vec();
    literal.resize(1 << 16, b'c');
    literal.push(b')');
    let mut hex = b"<".to_vec();
    hex.resize(1 << 16, b'4');
    hex.push(b'>');

    for input in [&name, &literal, &hex] {
        let limits = Limits::with(|s| s.memory_limit = Some(1 << 10));
        assert!(
            MaybeRef::<Object<'_>>::from_bytes_with_budget(input, &BudgetRef::new(&limits))
                .is_err()
        );
        assert!(limits.state().refused);

        let limits = Limits::default();
        assert!(matches!(
            MaybeRef::<Object<'_>>::from_bytes_with_budget(input, &BudgetRef::new(&limits)),
            Ok(Some(_))
        ));
        assert!(limits.state().memory >= input.len() / 2 - 2);
    }
}

#[test]
fn lazy_reads_charge_the_budget_after_the_first_read() {
    let mut input = b"<< /A (".to_vec();
    input.resize(1 << 20, b'x');
    input.extend_from_slice(b") /B [1 2 3] >>");
    let limits = Limits::default();
    let budget = BudgetRef::new(&limits);
    let dict = Dict::from_bytes_with_budget(&input, &budget)
        .unwrap()
        .unwrap();
    let after_read = limits.state().work;
    assert!(after_read >= input.len());

    // Reading an entry scans it again, and charges it again.
    assert!(dict.get::<hayro_syntax::object::String<'_>>("A").is_some());
    assert!(limits.state().work >= after_read + (1 << 20) - 16);

    // Once the operation is cancelled, later reads are refused too.
    limits.state().cancel_after_polls = Some(0);
    assert!(dict.get::<hayro_syntax::object::String<'_>>("A").is_none());
    assert!(dict.get::<Array<'_>>("B").is_none());
    assert!(limits.exhausted());
}

#[test]
fn a_stream_without_its_length_searches_for_endstream_within_the_budget() {
    let mut input = b"<< /Length 1 >> stream\n".to_vec();
    input.resize(8 << 20, b'z');
    input.extend_from_slice(b"\nendstream");

    let limits = Limits::default();
    assert!(matches!(
        MaybeRef::<Object<'_>>::from_bytes_with_budget(&input, &BudgetRef::new(&limits)),
        Ok(Some(_))
    ));

    let limits = Limits::with(|s| s.work_limit = Some(1 << 20));
    assert!(
        MaybeRef::<Object<'_>>::from_bytes_with_budget(&input, &BudgetRef::new(&limits)).is_err()
    );
    let s = limits.state();
    assert!(s.refused);
    assert!(s.work <= 1 << 20);
}
