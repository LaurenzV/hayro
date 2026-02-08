use alloc::boxed::Box;
use alloc::vec::Vec;

use hayro_postscript::{Object, Scanner};

use crate::{
    BfRange, CMap, CMapName, CharacterCollection, CidRange, CodespaceRange, MAX_NESTING_DEPTH,
    Metadata, Range, WritingMode,
};

struct Context<F> {
    buf: Vec<u8>,
    get_cmap: F,
}

pub(crate) fn parse<'a>(
    data: &[u8],
    get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    depth: u32,
) -> Option<CMap> {
    // Prevent stack overflow for malicious CMap files or circular references.
    if depth >= MAX_NESTING_DEPTH {
        return None;
    }

    let mut scanner = Scanner::new(data);
    let mut ctx = Context {
        buf: Vec::new(),
        get_cmap,
    };
    let mut codespace_ranges = Vec::new();
    let mut ranges = Vec::new();
    let mut notdef_ranges = Vec::new();
    let mut bf_entries = Vec::new();
    let mut base = None;

    let mut registry = None;
    let mut ordering = None;
    let mut supplement = None;
    let mut cmap_name = None;
    let mut writing_mode = None;
    let mut last_name: Option<Vec<u8>> = None;

    while !scanner.at_end() {
        let obj = scanner.parse_object().ok()?;

        let Object::Name(name) = &obj else { continue };

        if name.is_literal() {
            match name.as_str() {
                // Strictly speaking, Registry and Ordering should be strings,
                // but some PDF generators emit names instead, so be lenient
                // and try both.
                Some("Registry") => {
                    registry = parse_string_or_name(&mut scanner);
                }
                Some("Ordering") => {
                    ordering = parse_string_or_name(&mut scanner);
                }
                Some("Supplement") => {
                    supplement = scanner.parse_number().ok().map(|n| n.as_i32());
                }
                Some("CMapName") => {
                    cmap_name = scanner.parse_name().ok().and_then(|n| n.decode().ok());
                }
                Some("WMode") => {
                    writing_mode = parse_writing_mode(&mut scanner);
                }
                _ => {
                    last_name = name.decode().ok();
                }
            }
        } else {
            match name.as_str() {
                Some("begincodespacerange") => {
                    parse_codespace_range(&mut scanner, &mut codespace_ranges, &mut ctx)?;
                }
                Some("begincidrange") => {
                    parse_range(&mut scanner, &mut ranges, &mut ctx, "endcidrange")?;
                }
                Some("begincidchar") => {
                    parse_char(&mut scanner, &mut ranges, &mut ctx, "endcidchar")?;
                }
                Some("beginnotdefrange") => {
                    parse_range(&mut scanner, &mut notdef_ranges, &mut ctx, "endnotdefrange")?;
                }
                Some("beginnotdefchar") => {
                    parse_char(&mut scanner, &mut notdef_ranges, &mut ctx, "endnotdefchar")?;
                }
                Some("beginbfchar") => {
                    parse_bf_char(&mut scanner, &mut bf_entries, &mut ctx)?;
                }
                Some("beginbfrange") => {
                    parse_bf_range(&mut scanner, &mut bf_entries, &mut ctx)?;
                }
                Some("usecmap") => {
                    let nested_data = (ctx.get_cmap)(last_name.as_deref()?)?;

                    base = Some(Box::new(parse(
                        nested_data,
                        ctx.get_cmap.clone(),
                        depth + 1,
                    )?));
                }
                _ => {}
            }
        }
    }

    // Since we will use binary search for finding the correct entry, sort now.
    ranges.sort_by(|a, b| a.range.start.cmp(&b.range.start));
    notdef_ranges.sort_by(|a, b| a.range.start.cmp(&b.range.start));
    bf_entries.sort_by(|a, b| a.range.start.cmp(&b.range.start));

    // See PDFJS-3323, which has an invalid CIDSystemInfo entry.
    // We ignore it if it's invalid.
    let character_collection = if let (Some(registry), Some(ordering), Some(supplement)) =
        (registry, ordering, supplement)
    {
        Some(CharacterCollection {
            registry,
            ordering,
            supplement,
        })
    } else {
        None
    };

    let metadata = Metadata {
        character_collection,
        name: cmap_name,
        writing_mode,
    };

    Some(CMap {
        metadata,
        codespace_ranges,
        cid_ranges: ranges,
        notdef_ranges,
        bf_entries,
        base,
    })
}

fn parse_writing_mode(scanner: &mut Scanner<'_>) -> Option<WritingMode> {
    match scanner.parse_number().ok()?.as_i32() {
        0 => Some(WritingMode::Horizontal),
        1 => Some(WritingMode::Vertical),
        _ => None,
    }
}

fn parse_string_or_name(scanner: &mut Scanner<'_>) -> Option<Vec<u8>> {
    match scanner.parse_object().ok()? {
        Object::String(s) => s.decode().ok(),
        Object::Name(n) => n.decode().ok(),
        _ => None,
    }
}

fn parse_codespace_range<F>(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CodespaceRange>,
    ctx: &mut Context<F>,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if name_matches(&obj, "endcodespacerange") {
            return Some(());
        }

        let low = extract_u32_code(&obj, &mut ctx.buf)?;
        let n_bytes = u8::try_from(ctx.buf.len()).ok()?;
        let high = read_u32_code(scanner, &mut ctx.buf)?;

        if ctx.buf.len() != usize::from(n_bytes) {
            return None;
        }

        ranges.push(CodespaceRange {
            number_bytes: n_bytes,
            low,
            high,
        });
    }
}

fn parse_range<F>(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CidRange>,
    ctx: &mut Context<F>,
    end_marker: &str,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if name_matches(&obj, end_marker) {
            return Some(());
        }

        let start = extract_u32_code(&obj, &mut ctx.buf)?;
        let end = read_u32_code(scanner, &mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.parse_number().ok()?.as_i32()).ok()?;

        ranges.push(CidRange {
            range: Range { start, end },
            cid_start,
        });
    }
}

fn parse_char<F>(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CidRange>,
    ctx: &mut Context<F>,
    end_marker: &str,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if name_matches(&obj, end_marker) {
            return Some(());
        }

        let code = extract_u32_code(&obj, &mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.parse_number().ok()?.as_i32()).ok()?;

        ranges.push(CidRange {
            range: Range {
                start: code,
                end: code,
            },
            cid_start,
        });
    }
}

fn parse_bf_char<F>(
    scanner: &mut Scanner<'_>,
    entries: &mut Vec<BfRange>,
    ctx: &mut Context<F>,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if name_matches(&obj, "endbfchar") {
            return Some(());
        }

        let code = extract_u32_code(&obj, &mut ctx.buf)?;
        let dst = scanner.parse_string().ok()?;
        dst.decode_into(&mut ctx.buf).ok()?;

        entries.push(BfRange {
            range: Range {
                start: code,
                end: code,
            },
            dst_base: decode_be(&ctx.buf)?,
        });
    }
}

fn parse_bf_range<F>(
    scanner: &mut Scanner<'_>,
    entries: &mut Vec<BfRange>,
    ctx: &mut Context<F>,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if name_matches(&obj, "endbfrange") {
            return Some(());
        }

        let start = extract_u32_code(&obj, &mut ctx.buf)?;
        let end = read_u32_code(scanner, &mut ctx.buf)?;

        let next = scanner.parse_object().ok()?;

        match &next {
            Object::String(s) => {
                s.decode_into(&mut ctx.buf).ok()?;

                entries.push(BfRange {
                    range: Range { start, end },
                    dst_base: decode_be(&ctx.buf)?,
                });
            }
            Object::Array(array) => {
                let mut array_scanner = array.objects();

                for code in start..=end {
                    let s = array_scanner.parse_string().ok()?;
                    s.decode_into(&mut ctx.buf).ok()?;

                    entries.push(BfRange {
                        range: Range {
                            start: code,
                            end: code,
                        },
                        dst_base: decode_be(&ctx.buf)?,
                    });
                }
            }
            _ => return None,
        }
    }
}

/// Convert the buffer into native-endian u16, so that we can use `String::from_utf16`.
fn decode_be(bytes: &[u8]) -> Option<Vec<u16>> {
    if bytes.is_empty() {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len().div_ceil(2));
    let mut i = 0;

    // My understanding is that the bf strings should always be UTF16-BE encoded,
    // but it seems like some PDFs only have a single byte there? I guess we just
    // pad them? Wasn't able to find anything specific in the specifications.
    if !bytes.len().is_multiple_of(2) {
        out.push(u16::from(bytes[0]));
        i = 1;
    }

    while i < bytes.len() {
        out.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        i += 2;
    }

    Some(out)
}

fn read_u32_code(scanner: &mut Scanner<'_>, buf: &mut Vec<u8>) -> Option<u32> {
    let s = scanner.parse_string().ok()?;
    s.decode_into(buf).ok()?;
    bytes_to_u32(buf)
}

fn extract_u32_code(obj: &Object<'_>, buf: &mut Vec<u8>) -> Option<u32> {
    let Object::String(s) = obj else { return None };
    s.decode_into(buf).ok()?;
    bytes_to_u32(buf)
}

fn bytes_to_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }

    let mut val = 0_u32;
    for &b in bytes {
        val = (val << 8) | b as u32;
    }

    Some(val)
}

fn name_matches(obj: &Object<'_>, expected: &str) -> bool {
    matches!(obj, Object::Name(name) if !name.is_literal() && name.as_str() == Some(expected))
}
