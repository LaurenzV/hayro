use super::*;
use crate::object::Dict;

#[test]
fn body_probe_preserves_failures_ahead_of_main_cursor() {
    let input = b"<< /A << /B x 7 0 obj []";
    let mut cache = Containers::default();
    assert!(cache.resolve(Reader::new(input)).is_none());
    cache.evict_before(2);
    cache
        .skip_object(&mut Reader::new_with(input, input.len() - 2))
        .unwrap();
    assert!(cache.failed.contains(&6));
    cache.evict_before(7);
    assert!(cache.failed.is_empty());
}

#[test]
fn valid_children_and_failed_chains_respect_cache_bounds() {
    let padded_child = format!("[{}]", " ".repeat(64));
    let cases = [
        (
            "wide_empty_arrays",
            format!("[ {}]", "[] ".repeat(5000)),
            true,
        ),
        (
            "wide_padded_64b_arrays",
            format!("[ {}]", format!("{padded_child} ").repeat(5000)),
            true,
        ),
        (
            "wide_tiny_dicts",
            format!("[ {}]", "<< >> ".repeat(5000)),
            true,
        ),
        (
            "wide_dict",
            format!(
                "<< {} >>",
                (0..5000)
                    .map(|i| format!("/k{i} << /P ({}) >> ", "x".repeat(64)))
                    .collect::<String>()
            ),
            true,
        ),
        ("comment_obj", "[ % obj\n".repeat(5000), false),
        ("adjacent_dicts", "<< /A<<>>".repeat(5000), false),
        (
            "dense_balanced_arrays",
            format!("{}{}", "[".repeat(2000), "]".repeat(2000)),
            false,
        ),
        (
            "dense_unclosed_arrays",
            format!("{}]", "[".repeat(2000)),
            false,
        ),
        ("spaced_unclosed_arrays", "[ ".repeat(2000), false),
        ("contiguous_unclosed_dicts", "<<".repeat(2000), false),
        ("bare_obj_arrays", "[ obj ".repeat(2000), false),
        (
            "comment_object_word_arrays",
            "[ % object\n".repeat(2000),
            false,
        ),
        ("name_adjacent_dicts", "<< /A".repeat(2000), false),
    ];

    for (name, input, closed) in cases {
        let mut cache = Containers::default();
        assert_eq!(
            cache.resolve(Reader::new(input.as_bytes())).is_some(),
            closed,
            "{name}"
        );
        assert!(cache.failed.len() <= MAX_FAILED_CACHE, "{name}");
        if closed {
            assert!(cache.failed.is_empty(), "{name}");
        }
    }
}

#[test]
fn reachable_nested_containers_hit_cache_on_subsequent_probes() {
    const N: usize = 1024;
    let build_chain = |make_piece: &dyn Fn(usize) -> String, opener: &str| {
        let mut input = String::new();
        let mut offsets = Vec::with_capacity(N);
        for i in 1..=N {
            let piece = make_piece(i);
            let rel = piece.rfind(opener).unwrap();
            offsets.push(input.len() + rel);
            input.push_str(&piece);
        }
        (input, offsets)
    };

    let cases = [
        ("dict_key_nested", build_chain(&|_| "<< /A ".into(), "<<")),
        (
            "dict_array_nested",
            build_chain(&|_| "<< /A [ ".into(), "<<"),
        ),
        (
            "split_comment_array",
            build_chain(&|_| "% 1 0\nobj [ ".into(), "["),
        ),
        (
            "split_comment_dict",
            build_chain(&|_| "% 1 0\nobj<< ".into(), "<<"),
        ),
        (
            "split_literal",
            build_chain(&|_| "( 1 0 obj %)\n[ ".into(), "["),
        ),
        ("spaced_dicts", build_chain(&|_| "<< ".into(), "<<")),
        (
            "obj_spaced_array",
            build_chain(&|i| format!("{i} 0 obj [ "), "["),
        ),
        (
            "obj_adjacent_array",
            build_chain(&|i| format!("{i} 0 obj[ "), "["),
        ),
        (
            "obj_comment_array",
            build_chain(&|i| format!("{i} 0 obj%c\n[ "), "["),
        ),
        (
            "obj_spaced_dict",
            build_chain(&|i| format!("{i} 0 obj << "), "<<"),
        ),
        (
            "obj_adjacent_dict",
            build_chain(&|i| format!("{i} 0 obj<< "), "<<"),
        ),
        (
            "closed_dict_adjacent_dict",
            build_chain(&|_| "<< /A <<>>".into(), "<< /A"),
        ),
        (
            "comment_obj_array",
            build_chain(
                &|i| {
                    if i == 1 {
                        "1 0 obj [ ".into()
                    } else {
                        format!("% {i} 0 obj\n[ ")
                    }
                },
                "[",
            ),
        ),
        (
            "chained_obj_digit_adjacent_array",
            build_chain(
                &|i| {
                    let a = 2 * i - 1;
                    let b = 2 * i;
                    format!("{a} 0 obj{b} 0 obj[ ")
                },
                "[",
            ),
        ),
    ];

    for (name, (input, offsets)) in cases {
        let mut cache = Containers::default();
        let bytes = input.as_bytes();
        if matches!(
            name,
            "dict_key_nested" | "dict_array_nested" | "spaced_dicts" | "obj_spaced_array"
        ) {
            let mut first = Containers::default();
            assert!(first.resolve(Reader::new_with(bytes, offsets[0])).is_none());
            assert_eq!(first.failed.len(), MAX_OBJECT_NESTING_DEPTH, "{name}");
        }
        let mut misses = 0;
        for off in offsets {
            cache.evict_before(off);
            misses += usize::from(!cache.failed.contains(&off));
            assert!(
                cache.resolve(Reader::new_with(bytes, off)).is_none(),
                "{name}"
            );
        }
        assert!(misses <= 4, "{name}: {misses} misses");
    }
}

#[test]
fn depth_cutoff_preserves_valid_suffixes() {
    for (open, close) in [("[", "]"), ("<< /A ", ">>"), ("[<< /A ", ">>]")] {
        let stride = if open.starts_with("[<") { 2 } else { 1 };
        for depth in [511, 512, 513, 1024] {
            let n = depth / stride;
            let extra = depth % stride;
            let input = format!(
                "{}{}0{}{}",
                open.repeat(n),
                "[".repeat(extra),
                "]".repeat(extra),
                close.repeat(n)
            );
            let mut cache = Containers::default();
            for i in 0..n {
                let offset = i * open.len();
                cache.evict_before(offset);
                let value = cache.resolve(Reader::new_with(input.as_bytes(), offset));
                let height = depth - i * stride;
                assert_eq!(
                    value.map(|v| usize::from(v.height)),
                    (height <= MAX_OBJECT_NESTING_DEPTH).then_some(height)
                );
            }
        }
    }
}

#[test]
fn truncated_probe_preserves_complete_inner_container() {
    let depth = MAX_OBJECT_NESTING_DEPTH + 1;
    for inner in [
        format!("[{}]", " ".repeat(MAX_BATCH_BYTES + 1)),
        format!("[%{}\n]", "a".repeat(MAX_BATCH_BYTES + 1)),
        format!("[({})]", "x".repeat(MAX_BATCH_BYTES + 1)),
        format!("<< /{} 0 >>", "K".repeat(MAX_BATCH_BYTES)),
        format!("<<{}>>", " ".repeat(MAX_BATCH_BYTES - 1)),
    ] {
        let input = format!("{}{inner}{}", "[".repeat(depth - 1), "]".repeat(depth - 1));
        let mut cache = Containers::default();
        let reader = Reader::new(input.as_bytes());
        assert!(cache.resolve(reader).is_none());
        assert!(cache.failed.iter().copied().eq([0]));
        let suffix = cache
            .resolve(Reader::new_with(input.as_bytes(), 1))
            .unwrap();
        assert_eq!(usize::from(suffix.height), depth - 1);
        assert_eq!(suffix.end, input.len() - 1);
    }
}

#[test]
fn byte_budget_stops_before_distant_syntax_error() {
    let input = format!(
        "{}{}!",
        "[".repeat(MAX_OBJECT_NESTING_DEPTH + 1),
        " ".repeat(MAX_BATCH_BYTES)
    );
    let mut cache = Containers::default();
    assert!(cache.resolve(Reader::new(input.as_bytes())).is_none());
    assert!(cache.failed.iter().copied().eq([0]));
}

#[test]
fn batch_budget_stops_before_shallow_siblings_end() {
    let input = format!(
        "{}{}",
        "[".repeat(MAX_OBJECT_NESTING_DEPTH),
        "[]".repeat(10_000)
    );
    let mut cache = Containers::default();
    assert!(cache.resolve(Reader::new(input.as_bytes())).is_none());
    // Reaching EOF would prove every unclosed frame failed.
    assert!(cache.failed.iter().copied().eq([0]));
}

#[test]
fn cache_keeps_nearest_failures_when_full() {
    let input = "[".repeat(MAX_FAILED_CACHE + 2 * MAX_OBJECT_NESTING_DEPTH);
    let mut cache = Containers::default();
    for offset in (0..input.len()).step_by(MAX_OBJECT_NESTING_DEPTH).rev() {
        assert!(
            cache
                .resolve(Reader::new_with(input.as_bytes(), offset))
                .is_none()
        );
        assert!(cache.failed.len() <= MAX_FAILED_CACHE);
    }
    assert!(cache.failed.iter().copied().eq(0..MAX_FAILED_CACHE));

    assert!(
        cache
            .resolve(Reader::new_with(input.as_bytes(), MAX_FAILED_CACHE))
            .is_none()
    );
    assert!(cache.failed.iter().copied().eq(0..MAX_FAILED_CACHE));
}

#[test]
fn escaped_keys_match_dict() {
    let huge = format!("#41{}", "A".repeat(1024 * 1024));
    for key in [
        "#52oot",
        "Ty#70e",
        "#52#6f#6f#74",
        "#54#79#70#65",
        "Type#",
        "R#6Got",
        "RootX",
        "Root#20",
        "#2#52oot",
        "",
        &huge,
    ] {
        let input = format!("[ << /{key} 1 >> ]");
        let mut cache = Containers::default();
        assert!(cache.resolve(Reader::new(input.as_bytes())).is_some());
        let reader = Reader::new_with(input.as_bytes(), 2);
        let probe = cache.resolve(reader.clone()).unwrap();
        let dict = reader.clone().read_without_context::<Dict<'_>>().unwrap();
        assert_eq!(
            (probe.has_root, probe.has_type),
            (dict.contains_key(ROOT), dict.contains_key(TYPE)),
            "key={key}"
        );
    }

    // Delimiter-adjacent escaped keys.
    for input in [
        b"<< /#52oot[1] >>".as_slice(),
        b"<< /Ty#70e/Catalog >>".as_slice(),
        b"<< /#52oot<<>> >>".as_slice(),
        b"<< /#52oot(a) >>".as_slice(),
    ] {
        let mut cache = Containers::default();
        let reader = Reader::new(input);
        let probe = cache.resolve(reader.clone()).unwrap();
        let dict = reader.clone().read_without_context::<Dict<'_>>().unwrap();
        assert_eq!(
            (probe.has_root, probe.has_type),
            (dict.contains_key(ROOT), dict.contains_key(TYPE))
        );
    }

    // Escaped key right before EOF inside an unclosed dictionary.
    let mut cache = Containers::default();
    assert!(cache.resolve(Reader::new(b"<< /#52oot")).is_none());
}

#[test]
fn nested_results_match_fresh_readers() {
    for depth in [511, 512, 513] {
        let mut input = b"<< /Top ".to_vec();
        input.extend(core::iter::repeat_n(b'[', depth - 2));
        let inner = input.len();
        input.extend_from_slice(b"<< /R#6fot ([ ] << >>) % [ <<\n /Ty#70e /X >>");
        input.extend(core::iter::repeat_n(b']', depth - 2));
        input.extend_from_slice(b" >>");
        let mut cache = Containers::default();
        for offset in [0, 8, 9, inner] {
            let mut reader = Reader::new_with(&input, offset);
            let cached = cache.resolve(reader.clone());
            let skipped = reader.skip::<Object<'_>>(false);
            assert_eq!(cached.map(|c| c.end), skipped.map(|_| reader.offset()));
            if input[offset] == b'<' {
                reader.jump(offset);
                let dict = reader.read_without_context::<Dict<'_>>();
                assert_eq!(
                    cached.map(|c| (c.has_root, c.has_type)),
                    dict.map(|d| (d.contains_key(ROOT), d.contains_key(TYPE)))
                );
            }
        }
    }

    let inner_dict = format!("<< /Pad ({}) /R#6fot 1 /Ty#70e /Catalog >>", "x".repeat(64));
    let input = format!("<< /Outer {inner_dict} >>");
    let inner_offset = input.find(&inner_dict).unwrap();
    let mut cache = Containers::default();
    assert!(cache.resolve(Reader::new(input.as_bytes())).is_some());
    assert!(cache.failed.is_empty());
    let probe = cache
        .resolve(Reader::new_with(input.as_bytes(), inner_offset))
        .unwrap();
    let dict = Reader::new_with(input.as_bytes(), inner_offset)
        .read_without_context::<Dict<'_>>()
        .unwrap();
    assert_eq!(
        (probe.has_root, probe.has_type),
        (dict.contains_key(ROOT), dict.contains_key(TYPE))
    );
}
