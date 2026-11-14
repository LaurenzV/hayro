use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use crate::error::{HuffmanError, ParseError, Result, bail};
use crate::lazy::Lazy;
use crate::reader::Reader;
use crate::sync::Arc;

include!("huffman_tables_generated.rs");

const INLINE_TABLE_SIZE: usize = 43;

#[derive(Debug, Clone)]
pub(crate) struct HuffmanTable(Arc<InnerHuffmanTable>);

impl HuffmanTable {
    fn from_inline(nodes: [HuffmanNode; INLINE_TABLE_SIZE]) -> Self {
        Self(Arc::new(InnerHuffmanTable::Inline { nodes }))
    }

    fn from_dynamic(nodes: Vec<HuffmanNode>) -> Self {
        Self(Arc::new(InnerHuffmanTable::Dynamic { nodes }))
    }

    pub(crate) fn decode(&self, reader: &mut Reader<'_>) -> Result<Option<i32>> {
        let nodes: &[HuffmanNode] = match self.0.as_ref() {
            InnerHuffmanTable::Inline { nodes } => nodes,
            InnerHuffmanTable::Dynamic { nodes } => nodes,
        };

        HuffmanNode::decode_from(nodes, 0, reader)
    }

    pub(crate) fn decode_no_oob(&self, reader: &mut Reader<'_>) -> Result<i32> {
        Ok(self.decode(reader)?.ok_or(HuffmanError::UnexpectedOob)?)
    }

    pub(crate) fn build(lines: &[TableLine]) -> Result<Self> {
        let max_prefix_length = lines.iter().map(|l| l.prefix_length).max().unwrap_or(0);

        let mut nodes = vec![HuffmanNode::new_intermediate()];

        // Prefix lengths go up to 255, so codes are bit vectors; `None` once a code overflows its length.
        let mut current_code = Some(Vec::with_capacity(max_prefix_length as usize));

        for current_length in 1..=max_prefix_length {
            if let Some(code) = &mut current_code {
                code.push(false);
            }

            for line in lines {
                if line.prefix_length == current_length {
                    let code = current_code.as_mut().ok_or(HuffmanError::InvalidCode)?;
                    Self::insert_code(&mut nodes, code, line)?;

                    match code.iter().rposition(|&bit| !bit) {
                        Some(i) => {
                            code[i] = true;
                            code[i + 1..].fill(false);
                        }
                        None => current_code = None,
                    }
                }
            }
        }

        Ok(Self::from_dynamic(nodes))
    }

    pub(crate) fn build_uniform(num_symbols: u32, code_length: u32) -> Result<Self> {
        let lines: Vec<TableLine> = (0..num_symbols)
            .map(|i| TableLine::new(i as i32, code_length as u8, 0))
            .collect();
        Self::build(&lines)
    }

    fn insert_code(nodes: &mut Vec<HuffmanNode>, code: &[bool], line: &TableLine) -> Result<()> {
        let mut node_index = 0;

        for &bit in code {
            node_index = match nodes[node_index as usize].get_child(!bit) {
                Some(idx) => idx.get(),
                None => {
                    let new_idx = NonZeroU32::new(nodes.len() as u32).unwrap();
                    nodes.push(HuffmanNode::new_intermediate());
                    nodes[node_index as usize].set_child(!bit, new_idx)?;
                    new_idx.get()
                }
            };
        }

        nodes[node_index as usize] = HuffmanNode::new_leaf(
            line.range_low,
            line.range_length,
            line.is_lower,
            line.is_out_of_band,
        );

        Ok(())
    }

    pub(crate) fn read_custom(reader: &mut Reader<'_>) -> Result<Self> {
        let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

        let has_out_of_band = (flags & 1) != 0;
        let prefix_length_bits = ((flags >> 1) & 7) + 1;
        let range_length_bits = ((flags >> 4) & 7) + 1;

        let minimum_value = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;

        let maximum_value = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;

        let mut lines = Vec::new();
        let mut current_range_low = minimum_value;

        while current_range_low < maximum_value {
            let prefix_length = reader
                .read_bits(prefix_length_bits)
                .ok_or(HuffmanError::InvalidCode)? as u8;
            let range_length = reader
                .read_bits(range_length_bits)
                .ok_or(HuffmanError::InvalidCode)? as u8;

            lines.push(TableLine::new(
                current_range_low,
                prefix_length,
                range_length,
            ));

            let range_size = 1_i64
                .checked_shl(range_length as u32)
                .ok_or(HuffmanError::InvalidCode)?;
            let next_range_low = (current_range_low as i64)
                .checked_add(range_size)
                .ok_or(HuffmanError::InvalidCode)?;
            current_range_low =
                i32::try_from(next_range_low).map_err(|_| HuffmanError::InvalidCode)?;
        }

        lines.push(TableLine::lower(
            i64::from(minimum_value) - 1,
            reader
                .read_bits(prefix_length_bits)
                .ok_or(HuffmanError::InvalidCode)? as u8,
            32,
        ));

        lines.push(TableLine::upper(
            maximum_value,
            reader
                .read_bits(prefix_length_bits)
                .ok_or(HuffmanError::InvalidCode)? as u8,
            32,
        ));

        if has_out_of_band {
            lines.push(TableLine::oob(
                reader
                    .read_bits(prefix_length_bits)
                    .ok_or(HuffmanError::InvalidCode)? as u8,
            ));
        }

        Self::build(&lines)
    }
}

pub(crate) struct TableLine {
    pub(crate) range_low: i64,
    pub(crate) prefix_length: u8,
    pub(crate) range_length: u8,
    pub(crate) is_lower: bool,
    pub(crate) is_out_of_band: bool,
}

impl TableLine {
    pub(crate) const fn new(range_low: i32, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low: range_low as i64,
            prefix_length,
            range_length,
            is_lower: false,
            is_out_of_band: false,
        }
    }

    const fn lower(range_high: i64, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low: range_high,
            prefix_length,
            range_length,
            is_lower: true,
            is_out_of_band: false,
        }
    }

    const fn upper(range_low: i32, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low: range_low as i64,
            prefix_length,
            range_length,
            is_lower: false,
            is_out_of_band: false,
        }
    }

    const fn oob(prefix_length: u8) -> Self {
        Self {
            range_low: 0,
            prefix_length,
            range_length: 0,
            is_lower: false,
            is_out_of_band: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HuffmanNode {
    Intermediate {
        zero: Option<NonZeroU32>,
        one: Option<NonZeroU32>,
    },
    Leaf(LeafData),
    Empty,
}

impl HuffmanNode {
    fn new_intermediate() -> Self {
        Self::Intermediate {
            zero: None,
            one: None,
        }
    }

    fn new_leaf(range_low: i64, range_length: u8, is_lower: bool, is_out_of_band: bool) -> Self {
        Self::Leaf(LeafData {
            range_low,
            range_length,
            is_lower,
            is_out_of_band,
        })
    }

    fn get_child(&self, child_zero: bool) -> Option<NonZeroU32> {
        match self {
            Self::Intermediate { zero, one } => {
                if child_zero {
                    *zero
                } else {
                    *one
                }
            }
            _ => None,
        }
    }

    fn set_child(
        &mut self,
        child_zero: bool,
        index: NonZeroU32,
    ) -> core::result::Result<(), HuffmanError> {
        match self {
            Self::Intermediate { zero, one } => {
                if child_zero {
                    *zero = Some(index);
                } else {
                    *one = Some(index);
                }
                Ok(())
            }
            _ => Err(HuffmanError::ConflictingCodes),
        }
    }

    fn decode_from(
        nodes: &[Self],
        mut node_index: u32,
        reader: &mut Reader<'_>,
    ) -> Result<Option<i32>> {
        loop {
            match nodes[node_index as usize] {
                Self::Intermediate { zero, one } => {
                    let bit = reader.read_bit().ok_or(ParseError::UnexpectedEof)?;
                    let child_index = if bit == 0 { zero } else { one };
                    node_index = child_index.ok_or(HuffmanError::InvalidCode)?.get();
                }
                Self::Leaf(leaf) => {
                    if leaf.is_out_of_band {
                        return Ok(None);
                    }

                    let range_offset = i64::from(
                        reader
                            .read_bits(leaf.range_length)
                            .ok_or(HuffmanError::InvalidCode)?,
                    );

                    let value = if leaf.is_lower {
                        leaf.range_low - range_offset
                    } else {
                        leaf.range_low + range_offset
                    };

                    return Ok(Some(
                        i32::try_from(value).map_err(|_| HuffmanError::InvalidCode)?,
                    ));
                }
                Self::Empty => {
                    bail!(HuffmanError::InvalidCode);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LeafData {
    range_low: i64,
    range_length: u8,
    is_lower: bool,
    is_out_of_band: bool,
}

#[derive(Debug, Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "Inline variant is expected to be large."
)]
enum InnerHuffmanTable {
    Inline {
        nodes: [HuffmanNode; INLINE_TABLE_SIZE],
    },
    Dynamic {
        nodes: Vec<HuffmanNode>,
    },
}

#[derive(Debug)]
pub(crate) struct StandardHuffmanTables {
    table_a: Lazy<HuffmanTable>,
    table_b: Lazy<HuffmanTable>,
    table_c: Lazy<HuffmanTable>,
    table_d: Lazy<HuffmanTable>,
    table_e: Lazy<HuffmanTable>,
    table_f: Lazy<HuffmanTable>,
    table_g: Lazy<HuffmanTable>,
    table_h: Lazy<HuffmanTable>,
    table_i: Lazy<HuffmanTable>,
    table_j: Lazy<HuffmanTable>,
    table_k: Lazy<HuffmanTable>,
    table_l: Lazy<HuffmanTable>,
    table_m: Lazy<HuffmanTable>,
    table_n: Lazy<HuffmanTable>,
    table_o: Lazy<HuffmanTable>,
}

impl Default for StandardHuffmanTables {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardHuffmanTables {
    pub(crate) fn new() -> Self {
        Self {
            table_a: Lazy::new(|| HuffmanTable::from_inline(TABLE_A)),
            table_b: Lazy::new(|| HuffmanTable::from_inline(TABLE_B)),
            table_c: Lazy::new(|| HuffmanTable::from_inline(TABLE_C)),
            table_d: Lazy::new(|| HuffmanTable::from_inline(TABLE_D)),
            table_e: Lazy::new(|| HuffmanTable::from_inline(TABLE_E)),
            table_f: Lazy::new(|| HuffmanTable::from_inline(TABLE_F)),
            table_g: Lazy::new(|| HuffmanTable::from_inline(TABLE_G)),
            table_h: Lazy::new(|| HuffmanTable::from_inline(TABLE_H)),
            table_i: Lazy::new(|| HuffmanTable::from_inline(TABLE_I)),
            table_j: Lazy::new(|| HuffmanTable::from_inline(TABLE_J)),
            table_k: Lazy::new(|| HuffmanTable::from_inline(TABLE_K)),
            table_l: Lazy::new(|| HuffmanTable::from_inline(TABLE_L)),
            table_m: Lazy::new(|| HuffmanTable::from_inline(TABLE_M)),
            table_n: Lazy::new(|| HuffmanTable::from_inline(TABLE_N)),
            table_o: Lazy::new(|| HuffmanTable::from_inline(TABLE_O)),
        }
    }

    pub(crate) fn table_a(&self) -> &HuffmanTable {
        self.table_a.get(|| HuffmanTable::from_inline(TABLE_A))
    }

    pub(crate) fn table_b(&self) -> &HuffmanTable {
        self.table_b.get(|| HuffmanTable::from_inline(TABLE_B))
    }

    pub(crate) fn table_c(&self) -> &HuffmanTable {
        self.table_c.get(|| HuffmanTable::from_inline(TABLE_C))
    }

    pub(crate) fn table_d(&self) -> &HuffmanTable {
        self.table_d.get(|| HuffmanTable::from_inline(TABLE_D))
    }

    pub(crate) fn table_e(&self) -> &HuffmanTable {
        self.table_e.get(|| HuffmanTable::from_inline(TABLE_E))
    }

    pub(crate) fn table_f(&self) -> &HuffmanTable {
        self.table_f.get(|| HuffmanTable::from_inline(TABLE_F))
    }

    pub(crate) fn table_g(&self) -> &HuffmanTable {
        self.table_g.get(|| HuffmanTable::from_inline(TABLE_G))
    }

    pub(crate) fn table_h(&self) -> &HuffmanTable {
        self.table_h.get(|| HuffmanTable::from_inline(TABLE_H))
    }

    pub(crate) fn table_i(&self) -> &HuffmanTable {
        self.table_i.get(|| HuffmanTable::from_inline(TABLE_I))
    }

    pub(crate) fn table_j(&self) -> &HuffmanTable {
        self.table_j.get(|| HuffmanTable::from_inline(TABLE_J))
    }

    pub(crate) fn table_k(&self) -> &HuffmanTable {
        self.table_k.get(|| HuffmanTable::from_inline(TABLE_K))
    }

    pub(crate) fn table_l(&self) -> &HuffmanTable {
        self.table_l.get(|| HuffmanTable::from_inline(TABLE_L))
    }

    pub(crate) fn table_m(&self) -> &HuffmanTable {
        self.table_m.get(|| HuffmanTable::from_inline(TABLE_M))
    }

    pub(crate) fn table_n(&self) -> &HuffmanTable {
        self.table_n.get(|| HuffmanTable::from_inline(TABLE_N))
    }

    pub(crate) fn table_o(&self) -> &HuffmanTable {
        self.table_o.get(|| HuffmanTable::from_inline(TABLE_O))
    }
}
