//! Inspired by pdf.js, we use a custom binary format to optionally embed
//! the required cmaps in a space-efficient manner. However, we don't use
//! their format description but have a custom one. For more information,
//! see <https://github.com/LaurenzV/cmap-resources/tree/hayro>.

pub(crate) mod embedded;
pub(crate) mod huffman;
pub(crate) mod reader;

use alloc::boxed::Box;
use alloc::vec::Vec;

pub use embedded::load_embedded;

use crate::bcmap::embedded::BUNDLE;
use crate::{
    BfRange, CMap, CMapName, CharacterCollection, CidFamily, CidRange, CodespaceRange, Metadata,
    Range, WritingMode, parse,
};
use huffman::HuffmanTable;
use reader::Reader;

const BCMAP_MAGIC: &[u8] = b"bcmap";
const BCMAP_VERSION: u8 = 0x01;
const BCMAP_FILE_HEADER_SIZE: usize = 10;
const SEG_HEADER_SIZE: usize = 5;

const SEGMENT_RANGE_1B: u8 = 0x01;
const SEGMENT_SINGLE_1B: u8 = 0x02;
const SEGMENT_RANGE_2B: u8 = 0x03;
const SEGMENT_SINGLE_2B: u8 = 0x04;
const SEGMENT_RANGE_3B: u8 = 0x05;
const SEGMENT_SINGLE_3B: u8 = 0x06;
const SEGMENT_RANGE_4B: u8 = 0x07;
const SEGMENT_SINGLE_4B: u8 = 0x08;
const SEGMENT_USECMAP: u8 = 0x09;
const SEGMENT_NOTDEF: u8 = 0x0A;
const SEGMENT_WMODE: u8 = 0x0B;
const SEGMENT_CODESPACE: u8 = 0x0C;
const SEGMENT_NAME: u8 = 0x0D;
const SEGMENT_CID_SYSTEM_INFO: u8 = 0x0E;
const SEGMENT_BF_RANGE_VARIABLE: u8 = 0x0F;
const SEGMENT_BF_SINGLE_VARIABLE: u8 = 0x10;
const SEGMENT_BF_SINGLE_1U: u8 = 0x11;
const SEGMENT_BF_SINGLE_2U: u8 = 0x12;
const SEGMENT_BF_SINGLE_3U: u8 = 0x13;
const SEGMENT_BF_SINGLE_4U: u8 = 0x14;
const SEGMENT_BF_RANGE_1U: u8 = 0x15;
const SEGMENT_BF_RANGE_2U: u8 = 0x16;

pub(crate) fn parse<'a>(
    data: &[u8],
    get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    depth: u32,
) -> Option<CMap> {
    // While in theory we can assume that all binary cmaps are valid, it can
    // of course happen that an invalid one has been passed from outside, so
    // we still need to do proper validation.

    if data.get(..5)? != BCMAP_MAGIC || *data.get(5)? != BCMAP_VERSION {
        return None;
    }

    let mut reader = Reader::new(data);
    reader.read_bytes(6)?; // Skip magic + version.

    let file_len = reader.read_u32()? as usize;

    let delta_table = &BUNDLE.delta_table;
    let count_table = &BUNDLE.count_table;

    let mut cmap_name = None;
    let mut character_collection = None;
    let mut writing_mode = None;
    let mut base: Option<Box<CMap>> = None;
    let mut codespace_ranges = Vec::new();
    let mut cid_ranges = Vec::new();
    let mut notdef_ranges = Vec::new();
    let mut bf_entries = Vec::new();

    // Start parsing all segments of the file.
    let mut reader = Reader::new(data.get(BCMAP_FILE_HEADER_SIZE..file_len)?);

    while !reader.at_end() {
        let seg_type = reader.read_u8()?;
        let seg_len = reader.read_u32()? as usize;

        let payload = reader.read_bytes(seg_len.checked_sub(SEG_HEADER_SIZE)?)?;

        match seg_type {
            SEGMENT_NAME => {
                cmap_name = Some(Vec::from(payload));
            }
            SEGMENT_CID_SYSTEM_INFO => {
                // Format: Each string is 0-terminated.
                let mut r = Reader::new(payload);
                let registry = Vec::from(r.eat_until(|b| b == 0));
                r.read_u8()?;

                let ordering = Vec::from(r.eat_until(|b| b == 0));
                r.read_u8()?;
                let supplement = r.read_u16()? as i32;

                character_collection = Some(CharacterCollection {
                    family: CidFamily::from_registry_ordering(&registry, &ordering),
                    supplement,
                });
            }
            SEGMENT_USECMAP => {
                let base_data = get_cmap(CMapName::from_bytes(payload))?;

                base = Some(Box::new(parse::parse_inner(
                    base_data,
                    get_cmap.clone(),
                    depth,
                )?));
            }
            SEGMENT_WMODE => {
                writing_mode = match payload.first()? {
                    0 => Some(WritingMode::Horizontal),
                    1 => Some(WritingMode::Vertical),
                    _ => None,
                };
            }
            SEGMENT_CODESPACE => {
                parse_codespace(payload, &mut codespace_ranges)?;
            }
            SEGMENT_NOTDEF => {
                parse_notdef(payload, &mut notdef_ranges)?;
            }
            SEGMENT_RANGE_1B | SEGMENT_RANGE_2B | SEGMENT_RANGE_3B | SEGMENT_RANGE_4B => {
                parse_cid_segment(payload, &mut cid_ranges, delta_table, Some(count_table))?;
            }
            SEGMENT_SINGLE_1B | SEGMENT_SINGLE_2B | SEGMENT_SINGLE_3B | SEGMENT_SINGLE_4B => {
                parse_cid_segment(payload, &mut cid_ranges, delta_table, None)?;
            }
            SEGMENT_BF_RANGE_VARIABLE => {
                parse_bf_segment(
                    payload,
                    &mut bf_entries,
                    delta_table,
                    Some(count_table),
                    None,
                )?;
            }
            SEGMENT_BF_SINGLE_VARIABLE => {
                parse_bf_segment(payload, &mut bf_entries, delta_table, None, None)?;
            }
            SEGMENT_BF_RANGE_1U => {
                parse_bf_segment(
                    payload,
                    &mut bf_entries,
                    delta_table,
                    Some(count_table),
                    Some(1),
                )?;
            }
            SEGMENT_BF_RANGE_2U => {
                parse_bf_segment(
                    payload,
                    &mut bf_entries,
                    delta_table,
                    Some(count_table),
                    Some(2),
                )?;
            }
            SEGMENT_BF_SINGLE_1U => {
                parse_bf_segment(payload, &mut bf_entries, delta_table, None, Some(1))?;
            }
            SEGMENT_BF_SINGLE_2U => {
                parse_bf_segment(payload, &mut bf_entries, delta_table, None, Some(2))?;
            }
            SEGMENT_BF_SINGLE_3U => {
                parse_bf_segment(payload, &mut bf_entries, delta_table, None, Some(3))?;
            }
            SEGMENT_BF_SINGLE_4U => {
                parse_bf_segment(payload, &mut bf_entries, delta_table, None, Some(4))?;
            }
            _ => {
                return None;
            }
        }
    }

    cid_ranges.sort_by(|a, b| a.range.start.cmp(&b.range.start));
    notdef_ranges.sort_by(|a, b| a.range.start.cmp(&b.range.start));
    bf_entries.sort_by(|a, b| a.range.start.cmp(&b.range.start));

    Some(CMap {
        metadata: Metadata {
            character_collection,
            name: cmap_name,
            writing_mode,
        },
        codespace_ranges,
        cid_ranges,
        notdef_ranges,
        bf_entries,
        base,
    })
}

fn parse_codespace(payload: &[u8], ranges: &mut Vec<CodespaceRange>) -> Option<()> {
    let mut r = Reader::new(payload);
    let n_ranges = r.read_u8()? as usize;

    for _ in 0..n_ranges {
        let bw = r.read_u8()? as usize;
        let low = r.read_n_bytes(bw)?;
        let high = r.read_n_bytes(bw)?;

        ranges.push(CodespaceRange {
            number_bytes: bw as u8,
            low,
            high,
        });
    }

    Some(())
}

fn parse_notdef(payload: &[u8], ranges: &mut Vec<CidRange>) -> Option<()> {
    let mut r = Reader::new(payload);
    let bw = r.read_u8()? as usize;
    let n_entries = r.read_u16()? as usize;

    for _ in 0..n_entries {
        let start = r.read_n_bytes(bw)?;
        let end = r.read_n_bytes(bw)?;
        let cid = r.read_u16()? as u32;

        ranges.push(CidRange {
            range: Range { start, end },
            cid_start: cid,
        });
    }

    Some(())
}

fn parse_cid_segment(
    payload: &[u8],
    ranges: &mut Vec<CidRange>,
    delta_table: &HuffmanTable,
    count_table: Option<&HuffmanTable>,
) -> Option<()> {
    let mut r = Reader::new(payload);
    let n_entries = r.read_u16()? as usize;

    if n_entries == 0 {
        return Some(());
    }

    // There are two types of CID segments, all of which are stored in a columnar
    // fastion in the file:
    // For the first type, we have a simple mapping from a single code to a single
    // CID. In this case, the data stream first contains all codes using delta-coding,
    // encoded using huffman coding. This is followed by all CIDs, stored as
    // u16 (either as a raw CID, or as `0` in case the CID is +1 the previous CID).
    //
    // For the second type, we have a range of consecutive codes, which map
    // to a range of consecutive CIDs. In this case, we have an additional "count"
    // column that stores how many codes are mapped consecutively. These are also
    // encoded using huffman coding.

    // Read deltas for the start code points.
    let delta_len = r.read_u32()? as usize;
    let delta_data = r.read_bytes(delta_len)?;

    let mut delta_reader = Reader::new(delta_data);
    let mut deltas = Vec::with_capacity(n_entries);

    for _ in 0..n_entries {
        deltas.push(delta_table.decode(&mut delta_reader)?);
    }

    // Read the counts for each consecutive code range.
    let mut counts = Vec::new();
    if let Some(ct) = count_table {
        let count_len = r.read_u32()? as usize;
        let count_data = r.read_bytes(count_len)?;

        let mut count_reader = Reader::new(count_data);
        counts.reserve(n_entries);
        for _ in 0..n_entries {
            counts.push(ct.decode(&mut count_reader)?);
        }
    }

    // Read the CIDs and reconstruct the character codes that map to them.
    let is_range = count_table.is_some();
    let mut prev_end: Option<u32> = None;
    let mut prev_cid: Option<u32> = None;
    let mut prev_range_len: u32 = 0;

    for i in 0..n_entries {
        let raw_cid = r.read_u16()?;

        // Note that start deltas and counts are encoded minus one, so we
        // need to add one when reconstructing them.

        // Reconstruct start code.
        let start = if let Some(pe) = prev_end {
            pe + 1 + deltas[i]
        } else {
            deltas[i]
        };

        // Reconstruct end code.
        let end = if is_range {
            start + counts[i] + 1
        } else {
            start
        };

        // CID 0 means it's consecutive to the last seen CID, plus 1.
        let cid = if raw_cid == 0 {
            if let Some(pc) = prev_cid {
                pc + prev_range_len + 1
            } else {
                0
            }
        } else {
            raw_cid as u32
        };

        ranges.push(CidRange {
            range: Range { start, end },
            cid_start: cid,
        });

        prev_end = Some(end);
        prev_cid = Some(cid);
        prev_range_len = end - start;
    }

    Some(())
}

fn parse_bf_segment(
    payload: &[u8],
    entries: &mut Vec<BfRange>,
    delta_table: &HuffmanTable,
    count_table: Option<&HuffmanTable>,
    fixed_units: Option<usize>,
) -> Option<()> {
    let mut r = Reader::new(payload);
    let n_entries = r.read_u16()? as usize;

    let delta_len = r.read_u32()? as usize;
    let delta_data = r.read_bytes(delta_len)?;

    let mut delta_reader = Reader::new(delta_data);
    let mut deltas = Vec::with_capacity(n_entries);
    for _ in 0..n_entries {
        deltas.push(delta_table.decode(&mut delta_reader)?);
    }

    // Read counts for range segments.
    let mut counts = Vec::new();
    if let Some(ct) = count_table {
        let count_len = r.read_u32()? as usize;
        let count_data = r.read_bytes(count_len)?;

        let mut count_reader = Reader::new(count_data);
        counts.reserve(n_entries);
        for _ in 0..n_entries {
            counts.push(ct.decode(&mut count_reader)?);
        }
    }

    let is_range = count_table.is_some();
    let mut prev_end: Option<u32> = None;

    for i in 0..n_entries {
        // Fixed-width segments encode destinations without a length prefix.
        let n_units = match fixed_units {
            Some(nu) => nu,
            None => r.read_u8()? as usize,
        };
        let mut dst_base = Vec::with_capacity(n_units);
        for _ in 0..n_units {
            dst_base.push(r.read_u16()?);
        }

        let start = if let Some(pe) = prev_end {
            pe + 1 + deltas[i]
        } else {
            deltas[i]
        };

        let end = if is_range {
            start + counts[i] + 1
        } else {
            start
        };

        entries.push(BfRange {
            range: Range { start, end },
            dst_base,
        });

        prev_end = Some(end);
    }

    Some(())
}
