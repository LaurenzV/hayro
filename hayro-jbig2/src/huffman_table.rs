//! Huffman table decoding, described in Annex B.

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use crate::reader::Reader;

include!("huffman_tables_generated.rs");

/// Maximum number of nodes in an inline Huffman table.
const INLINE_TABLE_SIZE: usize = 43;

/// A queryable Huffman table.
#[derive(Debug, Clone)]
pub(crate) struct HuffmanTable(Rc<InnerHuffmanTable>);

impl HuffmanTable {
    /// Create a new inline Huffman table from a fixed-size node array.
    fn from_inline(nodes: [HuffmanNode; INLINE_TABLE_SIZE]) -> Self {
        Self(Rc::new(InnerHuffmanTable::Inline { nodes }))
    }

    /// Create a new dynamic Huffman table from a vector of nodes.
    fn from_dynamic(nodes: Vec<HuffmanNode>) -> Self {
        Self(Rc::new(InnerHuffmanTable::Dynamic { nodes }))
    }

    /// Decode a value from the bit reader using this Huffman table
    /// (B.4 "Using a Huffman table").
    ///
    /// Returns `Ok(None)` for out-of-band (OOB) values, `Ok(Some(value))` for decoded values.
    pub(crate) fn decode(&self, reader: &mut Reader<'_>) -> Result<Option<i32>, &'static str> {
        let nodes: &[HuffmanNode] = match self.0.as_ref() {
            InnerHuffmanTable::Inline { nodes } => nodes,
            InnerHuffmanTable::Dynamic { nodes } => nodes,
        };

        HuffmanNode::decode_from(nodes, 0, reader)
    }

    /// Build a Huffman table from table line definitions (B.3 "Assigning
    /// the prefix codes").
    pub(crate) fn build(lines: &[TableLine]) -> Self {
        // `NTEMP` - Number of table lines.
        let line_count = lines.len();

        // Step 1: "Build a histogram in the array LENCOUNT counting the number of times
        // each prefix length value occurs in PREFLEN: LENCOUNT[I] is the number of times
        // that the value I occurs in the array PREFLEN."
        // `LENMAX` - Maximum prefix length.
        let max_prefix_length = lines.iter().map(|l| l.prefix_length).max().unwrap_or(0) as usize;
        // `LENCOUNT` - Histogram of prefix lengths.
        let mut length_counts = vec![0_u32; max_prefix_length + 1];
        for line in lines {
            length_counts[line.prefix_length as usize] += 1;
        }

        // Step 2: "Let LENMAX be the largest value for which LENCOUNT[LENMAX] > 0. Set:
        // CURLEN = 1, FIRSTCODE[0] = 0, LENCOUNT[0] = 0"
        // `FIRSTCODE` - First code value for each length.
        let mut first_code_per_length = vec![0_u32; max_prefix_length + 1];
        // `CODES` - Assigned prefix codes for each line.
        let mut assigned_codes = vec![0_u32; line_count];
        length_counts[0] = 0;

        // Step 3: "While CURLEN ≤ LENMAX, perform the following operations:"
        // `CURLEN` - Current length being processed.
        for current_length in 1..=max_prefix_length {
            // a) "Set: FIRSTCODE[CURLEN] = (FIRSTCODE[CURLEN − 1] + LENCOUNT[CURLEN − 1]) × 2
            //         CURCODE = FIRSTCODE[CURLEN]
            //         CURTEMP = 0"
            first_code_per_length[current_length] =
                (first_code_per_length[current_length - 1] + length_counts[current_length - 1]) * 2;
            // `CURCODE` - Current code value being assigned.
            let mut current_code = first_code_per_length[current_length];

            // b) "While CURTEMP < NTEMP, perform the following operations:"
            // `CURTEMP` - Current line index.
            for line_index in 0..line_count {
                // i) "If PREFLEN[CURTEMP] = CURLEN, then set:
                //        CODES[CURTEMP] = CURCODE
                //        CURCODE = CURCODE + 1"
                if lines[line_index].prefix_length as usize == current_length {
                    assigned_codes[line_index] = current_code;
                    current_code += 1;
                }
                // ii) "Set CURTEMP = CURTEMP + 1" (implicit in for loop)
            }
            // c) "Set CURLEN = CURLEN + 1" (implicit in for loop)
        }

        // Build tree from assigned codes.
        let mut nodes = vec![HuffmanNode::new_intermediate()];

        for (i, line) in lines.iter().enumerate() {
            // "Note that the PREFLEN value 0 indicates that the table line is never used."
            if line.prefix_length == 0 {
                continue;
            }

            Self::insert_code(
                &mut nodes,
                0, // root index
                assigned_codes[i],
                line.prefix_length,
                line.range_low,
                line.range_length,
                line.is_lower,
                line.is_out_of_band,
            );
        }

        Self::from_dynamic(nodes)
    }

    /// Insert a code into the Huffman tree.
    fn insert_code(
        nodes: &mut Vec<HuffmanNode>,
        node_index: u32,
        code: u32,
        prefix_length: u8,
        range_low: i32,
        range_length: u8,
        is_lower: bool,
        is_out_of_band: bool,
    ) {
        if prefix_length == 0 {
            // We've consumed all bits, this should be a leaf.
            nodes[node_index as usize] =
                HuffmanNode::new_leaf(range_low, range_length, is_lower, is_out_of_band);
            return;
        }

        // Get the next bit (MSB first).
        let bit = (code >> (prefix_length - 1)) & 1;
        let remaining_code = code & ((1 << (prefix_length - 1)) - 1);

        let child_index = match nodes[node_index as usize].get_child(bit == 0) {
            Some(idx) => idx,
            None => {
                let new_idx = NonZeroU32::new(nodes.len() as u32).unwrap();
                nodes.push(HuffmanNode::new_intermediate());
                nodes[node_index as usize].set_child(bit == 0, new_idx);
                new_idx
            }
        };

        Self::insert_code(
            nodes,
            child_index.get(),
            remaining_code,
            prefix_length - 1,
            range_low,
            range_length,
            is_lower,
            is_out_of_band,
        );
    }

    /// Read a custom Huffman table from the bitstream (B.2 "Decoding a code table").
    pub(crate) fn read_custom(reader: &mut Reader<'_>) -> Result<Self, &'static str> {
        // 1) "Decode the code table flags field as described in B.2.1. This sets the values
        //    HTOOB, HTPS and HTRS."
        let flags = reader
            .read_byte()
            .ok_or("unexpected end of data reading huffman flags")?;

        // `HTOOB`
        let has_out_of_band = (flags & 1) != 0;
        // `HTPS`
        let prefix_length_bits = ((flags >> 1) & 7) + 1;
        // `HTRS`
        let range_length_bits = ((flags >> 4) & 7) + 1;

        // 2) "Decode the code table lowest value field as described in B.2.2. Let HTLOW be
        //    the value decoded."
        // `HTLOW`
        let minimum_value = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTLOW")?;

        // 3) "Decode the code table highest value field as described in B.2.3. Let HTHIGH be
        //    the value decoded."
        // `HTHIGH`
        let maximum_value = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTHIGH")?;

        // 4) "Set: CURRANGELOW = HTLOW, NTEMP = 0"
        let mut lines = Vec::new();
        // `CURRANGELOW`
        let mut current_range_low = minimum_value;

        // 5) "Decode each table line as follows:"
        //    d) "If CURRANGELOW ≥ HTHIGH then proceed to step 6."
        while current_range_low < maximum_value {
            // a) "Read HTPS bits. Set PREFLEN[NTEMP] to the value decoded."
            let prefix_length = reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8;
            // b) "Read HTRS bits. Let RANGELEN[NTEMP] be the value decoded."
            let range_length = reader
                .read_bits(range_length_bits)
                .ok_or("invalid huffman code")? as u8;

            // c) "Set: RANGELOW[NTEMP] = CURRANGELOW
            //         CURRANGELOW = CURRANGELOW + 2^RANGELEN[NTEMP]
            //         NTEMP = NTEMP + 1"
            lines.push(TableLine::new(
                current_range_low,
                prefix_length,
                range_length,
            ));

            let range_size = 1_i64
                .checked_shl(range_length as u32)
                .ok_or("range size overflow")?;
            let next_range_low = (current_range_low as i64)
                .checked_add(range_size)
                .ok_or("current_range_low overflow")?;
            current_range_low =
                i32::try_from(next_range_low).map_err(|_| "current_range_low out of i32 range")?;
        }

        // 6) "Read HTPS bits. Let LOWPREFLEN be the value read."
        // 7) "Set: PREFLEN[NTEMP] = LOWPREFLEN, RANGELEN[NTEMP] = 32,
        //         RANGELOW[NTEMP] = HTLOW − 1, NTEMP = NTEMP + 1
        //    This is the lower range table line for this table."
        lines.push(TableLine::lower(
            minimum_value - 1,
            reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8,
            32,
        ));

        // 8) "Read HTPS bits. Let HIGHPREFLEN be the value read."
        // 9) "Set: PREFLEN[NTEMP] = HIGHPREFLEN, RANGELEN[NTEMP] = 32,
        //         RANGELOW[NTEMP] = HTHIGH, NTEMP = NTEMP + 1
        //    This is the upper range table line for this table."
        lines.push(TableLine::upper(
            current_range_low,
            reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8,
            32,
        ));

        // 10) "If HTOOB is 1, then:
        //     a) Read HTPS bits. Let OOBPREFLEN be the value read.
        //     b) Set: PREFLEN[NTEMP] = OOBPREFLEN, NTEMP = NTEMP + 1
        //     This is the out-of-band table line for this table."
        if has_out_of_band {
            lines.push(TableLine::oob(
                reader
                    .read_bits(prefix_length_bits)
                    .ok_or("invalid huffman code")? as u8,
            ));
        }

        // 11) "Create the prefix codes using the algorithm described in B.3."
        Ok(Self::build(&lines))
    }
}

/// A table line definition used to build the Huffman tree.
pub(crate) struct TableLine {
    /// `RANGELOW` - The base value for computing the decoded value.
    /// For normal/upper lines: value = `range_low` + offset
    /// For lower lines: value = `range_low` - offset
    pub(crate) range_low: i32,
    /// `PREFLEN` - Prefix code length.
    pub(crate) prefix_length: u8,
    /// `RANGELEN` - Number of additional bits.
    pub(crate) range_length: u8,
    /// True if this is a lower range line (uses subtraction).
    pub(crate) is_lower: bool,
    /// `OOB` - True if this is the out-of-band marker.
    pub(crate) is_out_of_band: bool,
}

impl TableLine {
    /// Create a normal table line.
    pub(crate) const fn new(range_low: i32, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low,
            prefix_length,
            range_length,
            is_lower: false,
            is_out_of_band: false,
        }
    }

    /// Create a lower range line (-∞...`range_high`).
    const fn lower(range_high: i32, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low: range_high,
            prefix_length,
            range_length,
            is_lower: true,
            is_out_of_band: false,
        }
    }

    /// Create an upper range line (`range_low`...+∞).
    const fn upper(range_low: i32, prefix_length: u8, range_length: u8) -> Self {
        Self {
            range_low,
            prefix_length,
            range_length,
            is_lower: false,
            is_out_of_band: false,
        }
    }

    /// Create an out-of-band marker line.
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

/// A node in the Huffman tree.
#[derive(Debug, Clone, Copy)]
enum HuffmanNode {
    /// Intermediate node.
    Intermediate {
        zero: Option<NonZeroU32>,
        one: Option<NonZeroU32>,
    },
    /// Leaf node.
    Leaf(LeafData),
    /// Empty node (padding to fill fixed-size arrays in inline tables).
    Empty,
}

impl HuffmanNode {
    fn new_intermediate() -> Self {
        Self::Intermediate {
            zero: None,
            one: None,
        }
    }

    fn new_leaf(range_low: i32, range_length: u8, is_lower: bool, is_out_of_band: bool) -> Self {
        Self::Leaf(LeafData {
            range_low,
            range_length,
            is_lower,
            is_out_of_band,
        })
    }

    /// Get the child index for a given bit (0 or 1).
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

    /// Set the child index for a given bit (0 or 1).
    fn set_child(&mut self, child_zero: bool, index: NonZeroU32) {
        match self {
            Self::Intermediate { zero, one } => {
                if child_zero {
                    *zero = Some(index);
                } else {
                    *one = Some(index);
                }
            }
            _ => panic!("set_child called on non-intermediate node"),
        }
    }

    /// Implements B.4 "Using a Huffman table".
    fn decode_from(
        nodes: &[Self],
        mut node_index: u32,
        reader: &mut Reader<'_>,
    ) -> Result<Option<i32>, &'static str> {
        // 1) "Read one bit at a time until the bit string read matches the code assigned to
        //    one of the table lines."
        loop {
            match nodes[node_index as usize] {
                Self::Intermediate { zero, one } => {
                    let bit = reader
                        .read_bit()
                        .ok_or("unexpected end of data in huffman decode")?;
                    let child_index = if bit == 0 { zero } else { one };
                    node_index = child_index.ok_or("invalid huffman code")?.get();
                }
                Self::Leaf(leaf) => {
                    // 3) "If HTOOB is 1 for this table, and table line I is the out-of-band
                    //    table line for this table, then set: HTVAL = OOB"
                    if leaf.is_out_of_band {
                        return Ok(None);
                    }

                    // 2) "Read RANGELEN[I] bits. Let HTOFFSET be the value read."
                    // `HTOFFSET`
                    let range_offset = reader
                        .read_bits(leaf.range_length)
                        .ok_or("invalid huffman code")?
                        as i32;

                    // 4) "Otherwise, if table line I is the lower range table line for this
                    //    table, then set: HTVAL = RANGELOW[I] − HTOFFSET"
                    // 5) "Otherwise, set: HTVAL = RANGELOW[I] + HTOFFSET"
                    // `HTVAL`
                    let value = if leaf.is_lower {
                        leaf.range_low - range_offset
                    } else {
                        leaf.range_low + range_offset
                    };

                    return Ok(Some(value));
                }
                Self::Empty => {
                    return Err("invalid huffman code (empty node)");
                }
            }
        }
    }
}

/// Information stored at a leaf node of the Huffman tree.
#[derive(Debug, Clone, Copy)]
struct LeafData {
    /// `RANGELOW` - The base value for computing the decoded value.
    range_low: i32,
    /// `RANGELEN` - Number of additional bits to read.
    range_length: u8,
    /// True if this is a lower range line (uses subtraction).
    is_lower: bool,
    /// `OOB` - True if this is the out-of-band marker.
    is_out_of_band: bool,
}

/// The inner representation of a Huffman table.
///
/// This can be either an inline table (fixed-size array for standard tables)
/// or a dynamic table (Vec for runtime-built custom tables).
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

/// Standard Huffman tables (`TABLE_A` through `TABLE_O`).
#[derive(Debug)]
pub(crate) struct StandardHuffmanTables {
    table_a: HuffmanTable,
    table_b: HuffmanTable,
    table_c: HuffmanTable,
    table_d: HuffmanTable,
    table_e: HuffmanTable,
    table_f: HuffmanTable,
    table_g: HuffmanTable,
    table_h: HuffmanTable,
    table_i: HuffmanTable,
    table_j: HuffmanTable,
    table_k: HuffmanTable,
    table_l: HuffmanTable,
    table_m: HuffmanTable,
    table_n: HuffmanTable,
    table_o: HuffmanTable,
}

impl StandardHuffmanTables {
    /// Create a new instance with all tables initialized.
    pub(crate) fn new() -> Self {
        Self {
            table_a: HuffmanTable::from_inline(TABLE_A),
            table_b: HuffmanTable::from_inline(TABLE_B),
            table_c: HuffmanTable::from_inline(TABLE_C),
            table_d: HuffmanTable::from_inline(TABLE_D),
            table_e: HuffmanTable::from_inline(TABLE_E),
            table_f: HuffmanTable::from_inline(TABLE_F),
            table_g: HuffmanTable::from_inline(TABLE_G),
            table_h: HuffmanTable::from_inline(TABLE_H),
            table_i: HuffmanTable::from_inline(TABLE_I),
            table_j: HuffmanTable::from_inline(TABLE_J),
            table_k: HuffmanTable::from_inline(TABLE_K),
            table_l: HuffmanTable::from_inline(TABLE_L),
            table_m: HuffmanTable::from_inline(TABLE_M),
            table_n: HuffmanTable::from_inline(TABLE_N),
            table_o: HuffmanTable::from_inline(TABLE_O),
        }
    }

    /// Get Table B.1 (`TABLE_A`).
    pub(crate) fn table_a(&self) -> &HuffmanTable {
        &self.table_a
    }

    /// Get Table B.2 (`TABLE_B`).
    pub(crate) fn table_b(&self) -> &HuffmanTable {
        &self.table_b
    }

    /// Get Table B.3 (`TABLE_C`).
    pub(crate) fn table_c(&self) -> &HuffmanTable {
        &self.table_c
    }

    /// Get Table B.4 (`TABLE_D`).
    pub(crate) fn table_d(&self) -> &HuffmanTable {
        &self.table_d
    }

    /// Get Table B.5 (`TABLE_E`).
    pub(crate) fn table_e(&self) -> &HuffmanTable {
        &self.table_e
    }

    /// Get Table B.6 (`TABLE_F`).
    pub(crate) fn table_f(&self) -> &HuffmanTable {
        &self.table_f
    }

    /// Get Table B.7 (`TABLE_G`).
    pub(crate) fn table_g(&self) -> &HuffmanTable {
        &self.table_g
    }

    /// Get Table B.8 (`TABLE_H`).
    pub(crate) fn table_h(&self) -> &HuffmanTable {
        &self.table_h
    }

    /// Get Table B.9 (`TABLE_I`).
    pub(crate) fn table_i(&self) -> &HuffmanTable {
        &self.table_i
    }

    /// Get Table B.10 (`TABLE_J`).
    pub(crate) fn table_j(&self) -> &HuffmanTable {
        &self.table_j
    }

    /// Get Table B.11 (`TABLE_K`).
    pub(crate) fn table_k(&self) -> &HuffmanTable {
        &self.table_k
    }

    /// Get Table B.12 (`TABLE_L`).
    pub(crate) fn table_l(&self) -> &HuffmanTable {
        &self.table_l
    }

    /// Get Table B.13 (`TABLE_M`).
    pub(crate) fn table_m(&self) -> &HuffmanTable {
        &self.table_m
    }

    /// Get Table B.14 (`TABLE_N`).
    pub(crate) fn table_n(&self) -> &HuffmanTable {
        &self.table_n
    }

    /// Get Table B.15 (`TABLE_O`).
    pub(crate) fn table_o(&self) -> &HuffmanTable {
        &self.table_o
    }
}
