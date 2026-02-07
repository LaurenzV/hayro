use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::{Object, Scanner};

use crate::scanner_ext::ScannerExt;
use crate::{
    BfRange, CMap, CMapName, CidRange, CodespaceRange, MAX_NESTING_DEPTH, Metadata, WritingMode,
};

struct Context<F> {
    buf: Vec<u8>,
    // Used for converting UTF-16 to UTF-8.
    u16_buf: Vec<u16>,
    get_cmap: F,
}

impl<F> Context<F> {
    fn new(get_cmap: F) -> Self {
        Self {
            buf: Vec::new(),
            u16_buf: Vec::new(),
            get_cmap,
        }
    }
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
    let mut ctx = Context::new(get_cmap);
    let mut codespace_ranges = Vec::new();
    let mut ranges = Vec::new();
    let mut notdef_ranges = Vec::new();
    let mut bf_entries = Vec::new();
    let mut base = None;

    let mut registry = None;
    let mut ordering = None;
    let mut supplement = None;
    let mut cmap_name = None;
    let mut writing_mode = WritingMode::Horizontal;
    let mut last_name: Option<&str> = None;

    while !scanner.at_end() {
        let obj = scanner.parse_object().ok()?;

        let Object::Name(name) = &obj else { continue };

        if name.is_literal() {
            match name.as_str() {
                Some("Registry") => {
                    registry = Some(scanner.read_string(&mut ctx.buf)?);
                }
                Some("Ordering") => {
                    ordering = Some(scanner.read_string(&mut ctx.buf)?);
                }
                Some("Supplement") => {
                    supplement = Some(scanner.read_integer()?);
                }
                Some("CMapName") => {
                    cmap_name = Some(parse_cmap_name(&mut scanner)?);
                }
                Some("WMode") => {
                    writing_mode = parse_wmode(&mut scanner)?;
                }
                other => {
                    last_name = other;
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
                    let nested_data = (ctx.get_cmap)(last_name?.as_bytes())?;
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

    ranges.sort_by(|a, b| a.start.cmp(&b.start));
    notdef_ranges.sort_by(|a, b| a.start.cmp(&b.start));
    bf_entries.sort_by(|a, b| a.start.cmp(&b.start));

    let metadata = Metadata {
        registry: registry?,
        ordering: ordering?,
        supplement: supplement?,
        name: cmap_name?,
        writing_mode,
    };

    Some(CMap::new(
        metadata,
        codespace_ranges,
        ranges,
        notdef_ranges,
        bf_entries,
        base,
    ))
}

fn parse_cmap_name(scanner: &mut Scanner<'_>) -> Option<String> {
    let name = scanner.parse_name().ok()?;
    Some(String::from(name.as_str()?))
}

fn parse_wmode(scanner: &mut Scanner<'_>) -> Option<WritingMode> {
    let wmode = scanner.read_integer()?;
    match wmode {
        0 => Some(WritingMode::Horizontal),
        1 => Some(WritingMode::Vertical),
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

        if is_exec_name(&obj, "endcodespacerange") {
            return Some(());
        }

        let low = extract_u32_code(&obj, &mut ctx.buf)?;
        let n_bytes = u8::try_from(ctx.buf.len()).ok()?;
        let high = scanner.read_u32_code(&mut ctx.buf)?;

        if ctx.buf.len() != usize::from(n_bytes) {
            return None;
        }

        ranges.push(CodespaceRange { n_bytes, low, high });
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

        if is_exec_name(&obj, end_marker) {
            return Some(());
        }

        let start = extract_u32_code(&obj, &mut ctx.buf)?;
        let end = scanner.read_u32_code(&mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.read_integer()?).ok()?;

        ranges.push(CidRange {
            start,
            end,
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

        if is_exec_name(&obj, end_marker) {
            return Some(());
        }

        let code = extract_u32_code(&obj, &mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.read_integer()?).ok()?;

        ranges.push(CidRange {
            start: code,
            end: code,
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

        if is_exec_name(&obj, "endbfchar") {
            return Some(());
        }

        let code = extract_u32_code(&obj, &mut ctx.buf)?;
        let dst = scanner.parse_string().ok()?;
        dst.decode_into(&mut ctx.buf).ok()?;
        decode_be_into(&ctx.buf, &mut ctx.u16_buf)?;

        entries.push(BfRange {
            start: code,
            end: code,
            dst_base: ctx.u16_buf.clone(),
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

        if is_exec_name(&obj, "endbfrange") {
            return Some(());
        }

        let start = extract_u32_code(&obj, &mut ctx.buf)?;
        let end = scanner.read_u32_code(&mut ctx.buf)?;

        let next = scanner.parse_object().ok()?;

        match &next {
            Object::String(s) => {
                // Incrementing form: stored as a single entry.
                s.decode_into(&mut ctx.buf).ok()?;
                decode_be_into(&ctx.buf, &mut ctx.u16_buf)?;

                entries.push(BfRange {
                    start,
                    end,
                    dst_base: ctx.u16_buf.clone(),
                });
            }
            Object::Array(array) => {
                // Array form: each code maps to a specific dstString.
                let mut array_scanner = array.objects();
                for code in start..=end {
                    let s = array_scanner.parse_string().ok()?;
                    s.decode_into(&mut ctx.buf).ok()?;
                    decode_be_into(&ctx.buf, &mut ctx.u16_buf)?;

                    entries.push(BfRange {
                        start: code,
                        end: code,
                        dst_base: ctx.u16_buf.clone(),
                    });
                }
            }
            _ => return None,
        }
    }
}

/// Decode raw UTF-16BE bytes into a reusable `Vec<u16>` buffer.
fn decode_be_into(bytes: &[u8], out: &mut Vec<u16>) -> Option<()> {
    if bytes.len() < 2 || bytes.len() % 2 != 0 {
        return None;
    }

    out.clear();
    let mut i = 0;
    while i < bytes.len() {
        out.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        i += 2;
    }
    Some(())
}

fn extract_u32_code(obj: &Object<'_>, buf: &mut Vec<u8>) -> Option<u32> {
    let Object::String(s) = obj else { return None };
    s.decode_into(buf).ok()?;
    bytes_to_u32(buf)
}

fn bytes_to_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() > 4 {
        return None;
    }
    let mut val = 0u32;
    for &b in bytes {
        val = (val << 8) | u32::from(b);
    }
    Some(val)
}

fn is_exec_name(obj: &Object<'_>, expected: &str) -> bool {
    matches!(obj, Object::Name(name) if !name.is_literal() && name.as_str() == Some(expected))
}
