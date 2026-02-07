use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::{Object, Scanner};

use crate::ext::ScannerExt;
use crate::{CMap, CharacterCode, CidRange, Metadata, WritingMode};

struct Context {
    buf: Vec<u8>,
}

impl Context {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }
}

pub(crate) fn parse(data: &[u8]) -> Option<CMap> {
    let mut scanner = Scanner::new(data);
    let mut ctx = Context::new();
    let mut ranges = Vec::new();

    let mut registry = None;
    let mut ordering = None;
    let mut supplement = None;
    let mut cmap_name = None;
    let mut writing_mode = WritingMode::Horizontal;

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
                _ => {}
            }
        } else {
            match name.as_str() {
                Some("begincidrange") => {
                    parse_cid_range(&mut scanner, &mut ranges, &mut ctx)?;
                }
                Some("begincidchar") => {
                    parse_cid_char(&mut scanner, &mut ranges, &mut ctx)?;
                }
                _ => {}
            }
        }
    }

    ranges.sort_by(|a, b| a.start.cmp(&b.start));

    let metadata = Metadata {
        registry: registry?,
        ordering: ordering?,
        supplement: supplement?,
        name: cmap_name?,
        writing_mode,
    };

    Some(CMap::new(metadata, ranges))
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

fn parse_cid_range(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CidRange>,
    ctx: &mut Context,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if is_exec_name(&obj, "endcidrange") {
            return Some(());
        }

        let start = extract_char_code(&obj, &mut ctx.buf)?;
        let end = scanner.read_char_code(&mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.read_integer()?).ok()?;

        ranges.push(CidRange {
            start,
            end,
            cid_start,
        });
    }
}

fn parse_cid_char(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CidRange>,
    ctx: &mut Context,
) -> Option<()> {
    loop {
        let obj = scanner.parse_object().ok()?;

        if is_exec_name(&obj, "endcidchar") {
            return Some(());
        }

        let code = extract_char_code(&obj, &mut ctx.buf)?;
        let cid_start = u32::try_from(scanner.read_integer()?).ok()?;

        ranges.push(CidRange {
            start: code.clone(),
            end: code,
            cid_start,
        });
    }
}

fn extract_char_code(obj: &Object<'_>, buf: &mut Vec<u8>) -> Option<CharacterCode> {
    let Object::String(s) = obj else { return None };
    s.decode_into(buf).ok()?;
    Some(CharacterCode::from_bytes(buf))
}

fn is_exec_name(obj: &Object<'_>, expected: &str) -> bool {
    matches!(obj, Object::Name(name) if !name.is_literal() && name.as_str() == Some(expected))
}
