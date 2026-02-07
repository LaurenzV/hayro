use alloc::string::String;
use alloc::vec::Vec;

use hayro_postscript::{Object, Scanner};

use crate::{CMap, CharacterCode, CidRange, Metadata, WritingMode};

pub(crate) fn parse(data: &[u8]) -> Option<CMap> {
    let mut scanner = Scanner::new(data);
    let mut metadata = Metadata::default();
    let mut ranges = Vec::new();
    let mut buf = Vec::new();

    while let Some(result) = scanner.next() {
        // Skip unsupported PostScript features (dicts, procedures, etc.).
        let obj = match result {
            Ok(obj) => obj,
            Err(_) => continue,
        };

        let Object::Name(name) = &obj else { continue };

        if name.is_literal() {
            match name.as_str() {
                Some("Registry") => {
                    metadata.registry = Some(read_string(&mut scanner, &mut buf)?);
                }
                Some("Ordering") => {
                    metadata.ordering = Some(read_string(&mut scanner, &mut buf)?);
                }
                Some("Supplement") => {
                    metadata.supplement = Some(read_integer(&mut scanner)?);
                }
                Some("CMapName") => parse_cmap_name(&mut scanner, &mut metadata)?,
                Some("WMode") => parse_wmode(&mut scanner, &mut metadata)?,
                _ => {}
            }
        } else {
            match name.as_str() {
                Some("begincidrange") => {
                    parse_cid_range(&mut scanner, &mut ranges, &mut buf)?;
                }
                Some("begincidchar") => {
                    parse_cid_char(&mut scanner, &mut ranges, &mut buf)?;
                }
                _ => {}
            }
        }
    }

    ranges.sort_by(|a, b| a.start.cmp(&b.start));

    Some(CMap::new(metadata, ranges))
}

fn parse_cmap_name(scanner: &mut Scanner<'_>, metadata: &mut Metadata) -> Option<()> {
    let obj = scanner.next()?.ok()?;
    let Object::Name(name) = &obj else { return None };
    metadata.name = Some(String::from(name.as_str()?));
    Some(())
}

fn parse_wmode(scanner: &mut Scanner<'_>, metadata: &mut Metadata) -> Option<()> {
    let wmode = read_integer(scanner)?;
    metadata.writing_mode = match wmode {
        0 => WritingMode::Horizontal,
        1 => WritingMode::Vertical,
        _ => return None,
    };
    Some(())
}

fn parse_cid_range(
    scanner: &mut Scanner<'_>,
    ranges: &mut Vec<CidRange>,
    buf: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let obj = scanner.next()?.ok()?;

        if is_exec_name(&obj, "endcidrange") {
            return Some(());
        }

        let start = extract_char_code(&obj, buf)?;
        let end = read_char_code(scanner, buf)?;
        let cid_start = u32::try_from(read_integer(scanner)?).ok()?;

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
    buf: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let obj = scanner.next()?.ok()?;

        if is_exec_name(&obj, "endcidchar") {
            return Some(());
        }

        let code = extract_char_code(&obj, buf)?;
        let cid_start = u32::try_from(read_integer(scanner)?).ok()?;

        ranges.push(CidRange {
            start: code.clone(),
            end: code,
            cid_start,
        });
    }
}

fn read_string(scanner: &mut Scanner<'_>, buf: &mut Vec<u8>) -> Option<String> {
    let obj = scanner.next()?.ok()?;
    let Object::String(s) = &obj else { return None };
    s.decode_into(buf).ok()?;
    String::from_utf8(buf.to_vec()).ok()
}

fn read_integer(scanner: &mut Scanner<'_>) -> Option<i32> {
    let obj = scanner.next()?.ok()?;
    let Object::Number(n) = &obj else { return None };
    Some(n.as_i32())
}

fn read_char_code(scanner: &mut Scanner<'_>, buf: &mut Vec<u8>) -> Option<CharacterCode> {
    let obj = scanner.next()?.ok()?;
    extract_char_code(&obj, buf)
}

fn extract_char_code(obj: &Object<'_>, buf: &mut Vec<u8>) -> Option<CharacterCode> {
    let Object::String(s) = obj else { return None };
    s.decode_into(buf).ok()?;
    Some(CharacterCode::from_bytes(buf))
}

fn is_exec_name(obj: &Object<'_>, expected: &str) -> bool {
    matches!(obj, Object::Name(name) if !name.is_literal() && name.as_str() == Some(expected))
}
