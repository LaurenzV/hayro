use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::{Object, Scanner};

use crate::scanner_ext::ScannerExt;
use crate::{CMap, CMapName, CidRange, MAX_NESTING_DEPTH, Metadata, WritingMode};

struct Context<F> {
    buf: Vec<u8>,
    get_cmap: F,
}

impl<F> Context<F> {
    fn new(get_cmap: F) -> Self {
        Self {
            buf: Vec::new(),
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
    let mut ranges = Vec::new();
    let mut notdef_ranges = Vec::new();
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

    let metadata = Metadata {
        registry: registry?,
        ordering: ordering?,
        supplement: supplement?,
        name: cmap_name?,
        writing_mode,
    };

    Some(CMap::new(metadata, ranges, notdef_ranges, base))
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
