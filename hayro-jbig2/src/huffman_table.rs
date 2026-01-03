//! Huffman table decoding, described in Annex B.

use std::rc::Rc;

use crate::reader::Reader;

include!("huffman_tables_generated.rs");

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

/// Maximum number of nodes in an inline Huffman table.
const INLINE_TABLE_SIZE: usize = 43;

/// A node in the Huffman tree.
///
/// Nodes are stored in an arena (Vec or fixed-size array) and reference children by index.
#[derive(Debug, Clone, Copy)]
enum HuffmanNode {
    /// Intermediate node with two children (0 and 1 branches).
    /// Children are indices into the arena.
    Intermediate { zero: Option<u32>, one: Option<u32> },
    /// Leaf node containing the decoded value information.
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
    fn get_child(&self, bit: u32) -> Option<u32> {
        match self {
            HuffmanNode::Intermediate { zero, one } => {
                if bit == 0 {
                    *zero
                } else {
                    *one
                }
            }
            _ => None,
        }
    }

    /// Set the child index for a given bit (0 or 1).
    fn set_child(&mut self, bit: u32, index: u32) {
        match self {
            HuffmanNode::Intermediate { zero, one } => {
                if bit == 0 {
                    *zero = Some(index);
                } else {
                    *one = Some(index);
                }
            }
            _ => panic!("set_child called on non-intermediate node"),
        }
    }

    /// Decode from this node, returning the decoded value or None for OOB.
    fn decode_from(
        nodes: &[HuffmanNode],
        mut node_index: u32,
        reader: &mut Reader<'_>,
    ) -> Result<Option<i32>, &'static str> {
        loop {
            match nodes[node_index as usize] {
                HuffmanNode::Intermediate { zero, one } => {
                    let bit = reader
                        .read_bit()
                        .ok_or("unexpected end of data in huffman decode")?;
                    let child_index = if bit == 0 { zero } else { one };
                    node_index = child_index.ok_or("invalid huffman code")?;
                }
                HuffmanNode::Leaf(leaf) => {
                    if leaf.is_out_of_band {
                        return Ok(None);
                    }

                    let range_offset = reader
                        .read_bits(leaf.range_length)
                        .ok_or("invalid huffman code")?
                        as i32;

                    let value = if leaf.is_lower {
                        leaf.range_low - range_offset
                    } else {
                        leaf.range_low + range_offset
                    };

                    return Ok(Some(value));
                }
                HuffmanNode::Empty => {
                    return Err("invalid huffman code (empty node)");
                }
            }
        }
    }
}

/// The inner representation of a Huffman table.
///
/// This can be either an inline table (fixed-size array for standard tables)
/// or a dynamic table (Vec for runtime-built custom tables).
#[derive(Debug, Clone)]
enum InnerHuffmanTable {
    /// Inline table with a fixed-size node array.
    /// Used for standard tables (TABLE_A through TABLE_O).
    Inline {
        nodes: [HuffmanNode; INLINE_TABLE_SIZE],
    },
    /// Dynamic table with a Vec node array.
    /// Used for runtime-built custom tables.
    Dynamic { nodes: Vec<HuffmanNode> },
}

/// A Huffman table for JBIG2 decoding.
///
/// The table is represented as a binary tree stored in an arena,
/// where each path from root to leaf corresponds to a prefix code.
/// The root node is always at index 0.
///
/// This is a cheaply cloneable wrapper around the inner table representation
/// using reference counting.
#[derive(Debug, Clone)]
pub(crate) struct HuffmanTable(Rc<InnerHuffmanTable>);

impl HuffmanTable {
    /// Create a new inline Huffman table from a fixed-size node array.
    fn from_inline(nodes: [HuffmanNode; INLINE_TABLE_SIZE]) -> Self {
        Self(Rc::new(InnerHuffmanTable::Inline { nodes }))
    }

    /// Create a new dynamic Huffman table from a Vec of nodes.
    fn from_dynamic(nodes: Vec<HuffmanNode>) -> Self {
        Self(Rc::new(InnerHuffmanTable::Dynamic { nodes }))
    }

    /// Decode a value from the bit reader using this Huffman table.
    ///
    /// Implements B.4 "Using a Huffman table":
    /// 1) Read bits until matching a code
    /// 2) Read RANGELEN bits as HTOFFSET
    /// 3) If OOB line: return None
    /// 4) If lower range line: return RANGELOW - HTOFFSET
    /// 5) Otherwise: return RANGELOW + HTOFFSET
    ///
    /// Returns `Ok(None)` for out-of-band (OOB) values, `Ok(Some(value))` for decoded values.
    pub(crate) fn decode(&self, reader: &mut Reader<'_>) -> Result<Option<i32>, &'static str> {
        let nodes: &[HuffmanNode] = match self.0.as_ref() {
            InnerHuffmanTable::Inline { nodes } => nodes,
            InnerHuffmanTable::Dynamic { nodes } => nodes,
        };
        HuffmanNode::decode_from(nodes, 0, reader)
    }

    /// Build a Huffman table from table line definitions.
    ///
    /// This implements the algorithm from B.3 "Assigning the prefix codes".
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

        // Build tree from assigned codes using arena allocation.
        // "Note that the PREFLEN value 0 indicates that the table line is never used."
        let mut nodes = vec![HuffmanNode::new_intermediate()];

        for (i, line) in lines.iter().enumerate() {
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

    /// Insert a code into the Huffman tree arena.
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

        let child_index = match nodes[node_index as usize].get_child(bit) {
            Some(idx) => idx,
            None => {
                let new_idx = nodes.len() as u32;
                nodes.push(HuffmanNode::new_intermediate());
                nodes[node_index as usize].set_child(bit, new_idx);
                new_idx
            }
        };

        Self::insert_code(
            nodes,
            child_index,
            remaining_code,
            prefix_length - 1,
            range_low,
            range_length,
            is_lower,
            is_out_of_band,
        );
    }

    /// Read a custom Huffman table from the bitstream.
    ///
    /// Implements B.2 "Decoding a code table":
    /// 1) Read code table flags (1 byte): HTOOB (bit 0), HTPS-1 (bits 1-3), HTRS-1 (bits 4-6)
    /// 2) Read HTLOW (4 bytes, signed)
    /// 3) Read HTHIGH (4 bytes, signed)
    /// 4) Read table lines (PREFLEN as HTPS bits, RANGELEN as HTRS bits) until RANGELOW > HTHIGH
    /// 5) Read lower range line (PREFLEN only, RANGELEN=32 implied)
    /// 6) Read upper range line (PREFLEN only, RANGELEN=32 implied)
    /// 7) If HTOOB=1, read OOB line (PREFLEN only)
    pub(crate) fn read_custom(reader: &mut Reader<'_>) -> Result<Self, &'static str> {
        // Step 1: Read code table flags.
        let flags = reader
            .read_byte()
            .ok_or("unexpected end of data reading huffman flags")?;

        // `HTOOB` - "Bit 0 is HTOOB for this code table."
        let has_out_of_band = (flags & 1) != 0;
        // `HTPS` - "Bits 1-3 specify the value of HTPS – 1 for this code table."
        let prefix_length_bits = ((flags >> 1) & 7) + 1;
        // `HTRS` - "Bits 4-6 specify the value of HTRS – 1 for this code table."
        let range_length_bits = ((flags >> 4) & 7) + 1;

        // Step 2: Read HTLOW (lowest value in table).
        // `HTLOW` - The minimum value in the table.
        let minimum_value = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTLOW")?;

        // Step 3: Read HTHIGH (highest value in table).
        // `HTHIGH` - The maximum value in the table.
        let maximum_value = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTHIGH")?;

        // Step 4: Read table lines covering HTLOW to HTHIGH.
        // "Continue reading table lines... until CURRANGELOW > HTHIGH."
        let mut lines = Vec::new();
        // `CURRANGELOW` - Current range low value.
        let mut current_range_low = minimum_value;

        while current_range_low < maximum_value {
            let prefix_length = reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8;
            let range_length = reader
                .read_bits(range_length_bits)
                .ok_or("invalid huffman code")? as u8;

            lines.push(TableLine::new(
                current_range_low,
                prefix_length,
                range_length,
            ));

            // Advance to next range.
            // Range covers current_range_low to current_range_low + 2^range_length - 1.
            let range_size = 1_i64
                .checked_shl(range_length as u32)
                .ok_or("range size overflow")?;
            let next_range_low = (current_range_low as i64)
                .checked_add(range_size)
                .ok_or("current_range_low overflow")?;
            current_range_low =
                i32::try_from(next_range_low).map_err(|_| "current_range_low out of i32 range")?;
        }

        // Step 5: Read lower range line (-∞ to HTLOW-1).
        // Only PREFLEN is read; RANGELEN is implicitly 32.
        lines.push(TableLine::lower(
            minimum_value - 1,
            reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8,
            32,
        ));

        // Step 6: Read upper range line (current_range_low to +∞).
        // Only PREFLEN is read; RANGELEN is implicitly 32.
        lines.push(TableLine::upper(
            current_range_low,
            reader
                .read_bits(prefix_length_bits)
                .ok_or("invalid huffman code")? as u8,
            32,
        ));

        // Step 7: If HTOOB, read OOB line.
        if has_out_of_band {
            lines.push(TableLine::oob(
                reader
                    .read_bits(prefix_length_bits)
                    .ok_or("invalid huffman code")? as u8,
            ));
        }

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

/// Standard Huffman tables (TABLE_A through TABLE_O) for JBIG2 decoding.
///
/// All tables are initialized eagerly from precomputed inline data.
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
            table_a: HuffmanTable::from_inline(TABLE_A_NODES),
            table_b: HuffmanTable::from_inline(TABLE_B_NODES),
            table_c: HuffmanTable::from_inline(TABLE_C_NODES),
            table_d: HuffmanTable::from_inline(TABLE_D_NODES),
            table_e: HuffmanTable::from_inline(TABLE_E_NODES),
            table_f: HuffmanTable::from_inline(TABLE_F_NODES),
            table_g: HuffmanTable::from_inline(TABLE_G_NODES),
            table_h: HuffmanTable::from_inline(TABLE_H_NODES),
            table_i: HuffmanTable::from_inline(TABLE_I_NODES),
            table_j: HuffmanTable::from_inline(TABLE_J_NODES),
            table_k: HuffmanTable::from_inline(TABLE_K_NODES),
            table_l: HuffmanTable::from_inline(TABLE_L_NODES),
            table_m: HuffmanTable::from_inline(TABLE_M_NODES),
            table_n: HuffmanTable::from_inline(TABLE_N_NODES),
            table_o: HuffmanTable::from_inline(TABLE_O_NODES),
        }
    }

    /// Get Table B.1 (TABLE_A).
    pub(crate) fn table_a(&self) -> &HuffmanTable {
        &self.table_a
    }

    /// Get Table B.2 (TABLE_B).
    pub(crate) fn table_b(&self) -> &HuffmanTable {
        &self.table_b
    }

    /// Get Table B.3 (TABLE_C).
    pub(crate) fn table_c(&self) -> &HuffmanTable {
        &self.table_c
    }

    /// Get Table B.4 (TABLE_D).
    pub(crate) fn table_d(&self) -> &HuffmanTable {
        &self.table_d
    }

    /// Get Table B.5 (TABLE_E).
    pub(crate) fn table_e(&self) -> &HuffmanTable {
        &self.table_e
    }

    /// Get Table B.6 (TABLE_F).
    pub(crate) fn table_f(&self) -> &HuffmanTable {
        &self.table_f
    }

    /// Get Table B.7 (TABLE_G).
    pub(crate) fn table_g(&self) -> &HuffmanTable {
        &self.table_g
    }

    /// Get Table B.8 (TABLE_H).
    pub(crate) fn table_h(&self) -> &HuffmanTable {
        &self.table_h
    }

    /// Get Table B.9 (TABLE_I).
    pub(crate) fn table_i(&self) -> &HuffmanTable {
        &self.table_i
    }

    /// Get Table B.10 (TABLE_J).
    pub(crate) fn table_j(&self) -> &HuffmanTable {
        &self.table_j
    }

    /// Get Table B.11 (TABLE_K).
    pub(crate) fn table_k(&self) -> &HuffmanTable {
        &self.table_k
    }

    /// Get Table B.12 (TABLE_L).
    pub(crate) fn table_l(&self) -> &HuffmanTable {
        &self.table_l
    }

    /// Get Table B.13 (TABLE_M).
    pub(crate) fn table_m(&self) -> &HuffmanTable {
        &self.table_m
    }

    /// Get Table B.14 (TABLE_N).
    pub(crate) fn table_n(&self) -> &HuffmanTable {
        &self.table_n
    }

    /// Get Table B.15 (TABLE_O).
    pub(crate) fn table_o(&self) -> &HuffmanTable {
        &self.table_o
    }
}
