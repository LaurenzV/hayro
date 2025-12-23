//! Huffman table decoding for JBIG2.
//!
//! This module implements the standard Huffman tables defined in Annex B of
//! ITU-T T.88 (ISO/IEC 14492).

use std::sync::LazyLock;

use crate::reader::Reader;

/// Result of decoding a Huffman code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuffmanResult {
    /// A decoded integer value.
    Value(i32),
    /// Out-of-band marker (only possible when HTOOB=1).
    OutOfBand,
}

/// Information stored at a leaf node of the Huffman tree.
#[derive(Debug, Clone)]
struct LeafData {
    /// The base value for computing the decoded value.
    range_low: i32,
    /// Number of additional bits to read (RANGELEN).
    range_len: u8,
    /// True if this is a lower range line (uses subtraction).
    is_lower: bool,
    /// True if this is the out-of-band marker.
    is_oob: bool,
}

/// A node in the Huffman tree.
#[derive(Debug, Clone)]
enum HuffmanNode {
    /// Intermediate node with two children (0 and 1 branches).
    Intermediate {
        zero: Option<Box<HuffmanNode>>,
        one: Option<Box<HuffmanNode>>,
    },
    /// Leaf node containing the decoded value information.
    Leaf(LeafData),
}

impl HuffmanNode {
    fn new_intermediate() -> Self {
        Self::Intermediate {
            zero: None,
            one: None,
        }
    }

    fn new_leaf(range_low: i32, range_len: u8, is_lower: bool, is_oob: bool) -> Self {
        Self::Leaf(LeafData {
            range_low,
            range_len,
            is_lower,
            is_oob,
        })
    }
}

/// A Huffman table for JBIG2 decoding.
///
/// The table is represented as a binary tree where each path from root to
/// leaf corresponds to a prefix code.
#[derive(Debug, Clone)]
pub struct HuffmanTable {
    root: HuffmanNode,
}

/// A table line definition used to build the Huffman tree.
struct TableLine {
    /// The base value for computing the decoded value.
    /// For normal/upper lines: value = range_low + htoffset
    /// For lower lines: value = range_low - htoffset
    range_low: i32,
    /// Prefix code length (PREFLEN).
    preflen: u8,
    /// Number of additional bits (RANGELEN).
    range_len: u8,
    /// True if this is a lower range line (uses subtraction).
    is_lower: bool,
    /// True if this is the OOB marker.
    is_oob: bool,
}

impl TableLine {
    /// Create a normal table line.
    const fn new(range_low: i32, preflen: u8, range_len: u8) -> Self {
        Self {
            range_low,
            preflen,
            range_len,
            is_lower: false,
            is_oob: false,
        }
    }

    /// Create a lower range line (-∞...range_high).
    const fn lower(range_high: i32, preflen: u8, range_len: u8) -> Self {
        Self {
            range_low: range_high,
            preflen,
            range_len,
            is_lower: true,
            is_oob: false,
        }
    }

    /// Create an upper range line (range_low...+∞).
    const fn upper(range_low: i32, preflen: u8, range_len: u8) -> Self {
        Self {
            range_low,
            preflen,
            range_len,
            is_lower: false,
            is_oob: false,
        }
    }

    /// Create an out-of-band marker line.
    const fn oob(preflen: u8) -> Self {
        Self {
            range_low: 0,
            preflen,
            range_len: 0,
            is_lower: false,
            is_oob: true,
        }
    }
}

impl HuffmanTable {
    /// Build a Huffman table from table line definitions.
    ///
    /// This implements the algorithm from B.3 "Assigning the prefix codes".
    fn build(lines: &[TableLine]) -> Self {
        let ntemp = lines.len();

        // Step 1: "Build a histogram in the array LENCOUNT counting the number of times
        // each prefix length value occurs in PREFLEN: LENCOUNT[I] is the number of times
        // that the value I occurs in the array PREFLEN."
        let lenmax = lines.iter().map(|l| l.preflen).max().unwrap_or(0) as usize;
        let mut lencount = vec![0u32; lenmax + 1];
        for line in lines {
            lencount[line.preflen as usize] += 1;
        }

        // Step 2: "Let LENMAX be the largest value for which LENCOUNT[LENMAX] > 0. Set:
        // CURLEN = 1, FIRSTCODE[0] = 0, LENCOUNT[0] = 0"
        let mut firstcode = vec![0u32; lenmax + 1];
        let mut codes = vec![0u32; ntemp];
        lencount[0] = 0;

        // Step 3: "While CURLEN ≤ LENMAX, perform the following operations:"
        for curlen in 1..=lenmax {
            // a) "Set: FIRSTCODE[CURLEN] = (FIRSTCODE[CURLEN − 1] + LENCOUNT[CURLEN − 1]) × 2
            //         CURCODE = FIRSTCODE[CURLEN]
            //         CURTEMP = 0"
            firstcode[curlen] = (firstcode[curlen - 1] + lencount[curlen - 1]) * 2;
            let mut curcode = firstcode[curlen];

            // b) "While CURTEMP < NTEMP, perform the following operations:"
            for curtemp in 0..ntemp {
                // i) "If PREFLEN[CURTEMP] = CURLEN, then set:
                //        CODES[CURTEMP] = CURCODE
                //        CURCODE = CURCODE + 1"
                if lines[curtemp].preflen as usize == curlen {
                    codes[curtemp] = curcode;
                    curcode += 1;
                }
                // ii) "Set CURTEMP = CURTEMP + 1" (implicit in for loop)
            }
            // c) "Set CURLEN = CURLEN + 1" (implicit in for loop)
        }

        // Build tree from assigned codes.
        // "Note that the PREFLEN value 0 indicates that the table line is never used."
        let mut root = HuffmanNode::new_intermediate();
        for (i, line) in lines.iter().enumerate() {
            if line.preflen == 0 {
                continue;
            }

            Self::insert_code(
                &mut root,
                codes[i],
                line.preflen,
                line.range_low,
                line.range_len,
                line.is_lower,
                line.is_oob,
            );
        }

        Self { root }
    }

    /// Insert a code into the Huffman tree.
    fn insert_code(
        node: &mut HuffmanNode,
        code: u32,
        preflen: u8,
        range_low: i32,
        range_len: u8,
        is_lower: bool,
        is_oob: bool,
    ) {
        if preflen == 0 {
            // We've consumed all bits, this should be a leaf.
            *node = HuffmanNode::new_leaf(range_low, range_len, is_lower, is_oob);

            return;
        }

        // Get the next bit (MSB first).
        let bit = (code >> (preflen - 1)) & 1;
        let remaining_code = code & ((1 << (preflen - 1)) - 1);

        match node {
            HuffmanNode::Intermediate { zero, one } => {
                let child = if bit == 0 { zero } else { one };
                let child = child.get_or_insert_with(|| Box::new(HuffmanNode::new_intermediate()));

                Self::insert_code(
                    child,
                    remaining_code,
                    preflen - 1,
                    range_low,
                    range_len,
                    is_lower,
                    is_oob,
                );
            }
            HuffmanNode::Leaf(_) => {
                panic!("attempted to insert code into leaf node");
            }
        }
    }

    /// Decode a value from the bit reader using this Huffman table.
    ///
    /// Implements B.4 "Using a Huffman table":
    /// 1) Read bits until matching a code
    /// 2) Read RANGELEN bits as HTOFFSET
    /// 3) If OOB line: return OOB
    /// 4) If lower range line: return RANGELOW - HTOFFSET (we use range_high as the base)
    /// 5) Otherwise: return RANGELOW + HTOFFSET
    pub fn decode(&self, reader: &mut Reader<'_>) -> Result<HuffmanResult, &'static str> {
        let mut node = &self.root;

        loop {
            match node {
                HuffmanNode::Intermediate { zero, one } => {
                    let bit = reader
                        .read_bit()
                        .ok_or("unexpected end of data in huffman decode")?;
                    let child = if bit == 0 { zero } else { one };
                    node = child.as_ref().ok_or("invalid huffman code")?.as_ref();
                }
                HuffmanNode::Leaf(leaf) => {
                    if leaf.is_oob {
                        return Ok(HuffmanResult::OutOfBand);
                    }

                    let htoffset = reader.read_bits(leaf.range_len)? as i32;

                    let value = if leaf.is_lower {
                        leaf.range_low - htoffset
                    } else {
                        leaf.range_low + htoffset
                    };

                    return Ok(HuffmanResult::Value(value));
                }
            }
        }
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
    pub fn read_custom(reader: &mut Reader<'_>) -> Result<Self, &'static str> {
        // Step 1: Read code table flags.
        let flags = reader
            .read_byte()
            .ok_or("unexpected end of data reading huffman flags")?;

        // "Bit 0 is HTOOB for this code table."
        let htoob = (flags & 1) != 0;
        // "Bits 1-3 specify the value of HTPS – 1 for this code table."
        let htps = ((flags >> 1) & 7) + 1;
        // "Bits 4-6 specify the value of HTRS – 1 for this code table."
        let htrs = ((flags >> 4) & 7) + 1;

        // Step 2: Read HTLOW (lowest value in table).
        let htlow = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTLOW")?;

        // Step 3: Read HTHIGH (highest value in table).
        let hthigh = reader
            .read_i32()
            .ok_or("unexpected end of data reading HTHIGH")?;

        // Step 4: Read table lines covering HTLOW to HTHIGH.
        // "Continue reading table lines... until CURRANGELOW > HTHIGH."
        let mut lines = Vec::new();
        let mut currangelow = htlow;

        while currangelow < hthigh {
            let preflen = reader.read_bits(htps)? as u8;
            let rangelen = reader.read_bits(htrs)? as u8;

            lines.push(TableLine::new(currangelow, preflen, rangelen));

            // Advance to next range.
            // Range covers currangelow to currangelow + 2^rangelen - 1.
            let range_size = 1i64
                .checked_shl(rangelen as u32)
                .ok_or("range size overflow")?;
            let next = (currangelow as i64)
                .checked_add(range_size)
                .ok_or("currangelow overflow")?;
            currangelow = i32::try_from(next).map_err(|_| "currangelow out of i32 range")?;
        }

        // Step 5: Read lower range line (-∞ to HTLOW-1).
        // Only PREFLEN is read; RANGELEN is implicitly 32.
        lines.push(TableLine::lower(
            htlow - 1,
            reader.read_bits(htps)? as u8,
            32,
        ));

        // Step 6: Read upper range line (currangelow to +∞).
        // Only PREFLEN is read; RANGELEN is implicitly 32.
        lines.push(TableLine::upper(
            currangelow,
            reader.read_bits(htps)? as u8,
            32,
        ));

        // Step 7: If HTOOB, read OOB line.
        if htoob {
            lines.push(TableLine::oob(reader.read_bits(htps)? as u8));
        }

        Ok(Self::build(&lines))
    }
}

/// Table B.1 – Standard Huffman table A (HTOOB = 0)
pub static TABLE_A: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(0, 1, 4),        // 0...15
        TableLine::new(16, 2, 8),       // 16...271
        TableLine::new(272, 3, 16),     // 272...65807
        TableLine::upper(65808, 3, 32), // 65808...∞
    ])
});

/// Table B.2 – Standard Huffman table B (HTOOB = 1)
pub static TABLE_B: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(0, 1, 0),     // 0
        TableLine::new(1, 2, 0),     // 1
        TableLine::new(2, 3, 0),     // 2
        TableLine::new(3, 4, 3),     // 3...10
        TableLine::new(11, 5, 6),    // 11...74
        TableLine::upper(75, 6, 32), // 75...∞
        TableLine::oob(6),           // OOB
    ])
});

/// Table B.3 – Standard Huffman table C (HTOOB = 1)
pub static TABLE_C: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-256, 8, 8),    // -256...-1
        TableLine::new(0, 1, 0),       // 0
        TableLine::new(1, 2, 0),       // 1
        TableLine::new(2, 3, 0),       // 2
        TableLine::new(3, 4, 3),       // 3...10
        TableLine::new(11, 5, 6),      // 11...74
        TableLine::lower(-257, 8, 32), // -∞...-257
        TableLine::upper(75, 7, 32),   // 75...∞
        TableLine::oob(6),             // OOB
    ])
});

/// Table B.4 – Standard Huffman table D (HTOOB = 0)
pub static TABLE_D: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(1, 1, 0),     // 1
        TableLine::new(2, 2, 0),     // 2
        TableLine::new(3, 3, 0),     // 3
        TableLine::new(4, 4, 3),     // 4...11
        TableLine::new(12, 5, 6),    // 12...75
        TableLine::upper(76, 5, 32), // 76...∞
    ])
});

/// Table B.5 – Standard Huffman table E (HTOOB = 0)
pub static TABLE_E: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-255, 7, 8),    // -255...0
        TableLine::new(1, 1, 0),       // 1
        TableLine::new(2, 2, 0),       // 2
        TableLine::new(3, 3, 0),       // 3
        TableLine::new(4, 4, 3),       // 4...11
        TableLine::new(12, 5, 6),      // 12...75
        TableLine::lower(-256, 7, 32), // -∞...-256
        TableLine::upper(76, 6, 32),   // 76...∞
    ])
});

/// Table B.6 – Standard Huffman table F (HTOOB = 0)
pub static TABLE_F: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-2048, 5, 10),   // -2048...-1025
        TableLine::new(-1024, 4, 9),    // -1024...-513
        TableLine::new(-512, 4, 8),     // -512...-257
        TableLine::new(-256, 4, 7),     // -256...-129
        TableLine::new(-128, 5, 6),     // -128...-65
        TableLine::new(-64, 5, 5),      // -64...-33
        TableLine::new(-32, 4, 5),      // -32...-1
        TableLine::new(0, 2, 7),        // 0...127
        TableLine::new(128, 3, 7),      // 128...255
        TableLine::new(256, 3, 8),      // 256...511
        TableLine::new(512, 4, 9),      // 512...1023
        TableLine::new(1024, 4, 10),    // 1024...2047
        TableLine::lower(-2049, 6, 32), // -∞...-2049
        TableLine::upper(2048, 6, 32),  // 2048...∞
    ])
});

/// Table B.7 – Standard Huffman table G (HTOOB = 0)
pub static TABLE_G: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-1024, 4, 9),    // -1024...-513
        TableLine::new(-512, 3, 8),     // -512...-257
        TableLine::new(-256, 4, 7),     // -256...-129
        TableLine::new(-128, 5, 6),     // -128...-65
        TableLine::new(-64, 5, 5),      // -64...-33
        TableLine::new(-32, 4, 5),      // -32...-1
        TableLine::new(0, 4, 5),        // 0...31
        TableLine::new(32, 5, 5),       // 32...63
        TableLine::new(64, 5, 6),       // 64...127
        TableLine::new(128, 4, 7),      // 128...255
        TableLine::new(256, 3, 8),      // 256...511
        TableLine::new(512, 3, 9),      // 512...1023
        TableLine::new(1024, 3, 10),    // 1024...2047
        TableLine::lower(-1025, 5, 32), // -∞...-1025
        TableLine::upper(2048, 5, 32),  // 2048...∞
    ])
});

/// Table B.8 – Standard Huffman table H (HTOOB = 1)
pub static TABLE_H: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-15, 8, 3),     // -15...-8
        TableLine::new(-7, 9, 1),      // -7...-6
        TableLine::new(-5, 8, 1),      // -5...-4
        TableLine::new(-3, 9, 0),      // -3
        TableLine::new(-2, 7, 0),      // -2
        TableLine::new(-1, 4, 0),      // -1
        TableLine::new(0, 2, 1),       // 0...1
        TableLine::new(2, 5, 0),       // 2
        TableLine::new(3, 6, 0),       // 3
        TableLine::new(4, 3, 4),       // 4...19
        TableLine::new(20, 6, 1),      // 20...21
        TableLine::new(22, 4, 4),      // 22...37
        TableLine::new(38, 4, 5),      // 38...69
        TableLine::new(70, 5, 6),      // 70...133
        TableLine::new(134, 5, 7),     // 134...261
        TableLine::new(262, 6, 7),     // 262...389
        TableLine::new(390, 7, 8),     // 390...645
        TableLine::new(646, 6, 10),    // 646...1669
        TableLine::lower(-16, 9, 32),  // -∞...-16
        TableLine::upper(1670, 9, 32), // 1670...∞
        TableLine::oob(2),             // OOB
    ])
});

/// Table B.9 – Standard Huffman table I (HTOOB = 1)
pub static TABLE_I: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-31, 8, 4),     // -31...-16
        TableLine::new(-15, 9, 2),     // -15...-12
        TableLine::new(-11, 8, 2),     // -11...-8
        TableLine::new(-7, 9, 1),      // -7...-6
        TableLine::new(-5, 7, 1),      // -5...-4
        TableLine::new(-3, 4, 1),      // -3...-2
        TableLine::new(-1, 3, 1),      // -1...0
        TableLine::new(1, 3, 1),       // 1...2
        TableLine::new(3, 5, 1),       // 3...4
        TableLine::new(5, 6, 1),       // 5...6
        TableLine::new(7, 3, 5),       // 7...38
        TableLine::new(39, 6, 2),      // 39...42
        TableLine::new(43, 4, 5),      // 43...74
        TableLine::new(75, 4, 6),      // 75...138
        TableLine::new(139, 5, 7),     // 139...266
        TableLine::new(267, 5, 8),     // 267...522
        TableLine::new(523, 6, 8),     // 523...778
        TableLine::new(779, 7, 9),     // 779...1290
        TableLine::new(1291, 6, 11),   // 1291...3338
        TableLine::lower(-32, 9, 32),  // -∞...-32
        TableLine::upper(3339, 9, 32), // 3339...∞
        TableLine::oob(2),             // OOB
    ])
});

/// Table B.10 – Standard Huffman table J (HTOOB = 1)
pub static TABLE_J: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-21, 7, 4),     // -21...-6
        TableLine::new(-5, 8, 0),      // -5
        TableLine::new(-4, 7, 0),      // -4
        TableLine::new(-3, 5, 0),      // -3
        TableLine::new(-2, 2, 2),      // -2...1
        TableLine::new(2, 5, 0),       // 2
        TableLine::new(3, 6, 0),       // 3
        TableLine::new(4, 7, 0),       // 4
        TableLine::new(5, 8, 0),       // 5
        TableLine::new(6, 2, 6),       // 6...69
        TableLine::new(70, 5, 5),      // 70...101
        TableLine::new(102, 6, 5),     // 102...133
        TableLine::new(134, 6, 6),     // 134...197
        TableLine::new(198, 6, 7),     // 198...325
        TableLine::new(326, 6, 8),     // 326...581
        TableLine::new(582, 6, 9),     // 582...1093
        TableLine::new(1094, 6, 10),   // 1094...2117
        TableLine::new(2118, 7, 11),   // 2118...4165
        TableLine::lower(-22, 8, 32),  // -∞...-22
        TableLine::upper(4166, 8, 32), // 4166...∞
        TableLine::oob(2),             // OOB
    ])
});

/// Table B.11 – Standard Huffman table K (HTOOB = 0)
pub static TABLE_K: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(1, 1, 0),      // 1
        TableLine::new(2, 2, 1),      // 2...3
        TableLine::new(4, 4, 0),      // 4
        TableLine::new(5, 4, 1),      // 5...6
        TableLine::new(7, 5, 1),      // 7...8
        TableLine::new(9, 5, 2),      // 9...12
        TableLine::new(13, 6, 2),     // 13...16
        TableLine::new(17, 7, 2),     // 17...20
        TableLine::new(21, 7, 3),     // 21...28
        TableLine::new(29, 7, 4),     // 29...44
        TableLine::new(45, 7, 5),     // 45...76
        TableLine::new(77, 7, 6),     // 77...140
        TableLine::upper(141, 7, 32), // 141...∞
    ])
});

/// Table B.12 – Standard Huffman table L (HTOOB = 0)
pub static TABLE_L: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(1, 1, 0),     // 1
        TableLine::new(2, 2, 0),     // 2
        TableLine::new(3, 3, 1),     // 3...4
        TableLine::new(5, 5, 0),     // 5
        TableLine::new(6, 5, 1),     // 6...7
        TableLine::new(8, 6, 1),     // 8...9
        TableLine::new(10, 7, 0),    // 10
        TableLine::new(11, 7, 1),    // 11...12
        TableLine::new(13, 7, 2),    // 13...16
        TableLine::new(17, 7, 3),    // 17...24
        TableLine::new(25, 7, 4),    // 25...40
        TableLine::new(41, 8, 5),    // 41...72
        TableLine::upper(73, 8, 32), // 73...∞
    ])
});

/// Table B.13 – Standard Huffman table M (HTOOB = 0)
pub static TABLE_M: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(1, 1, 0),      // 1
        TableLine::new(2, 3, 0),      // 2
        TableLine::new(3, 4, 0),      // 3
        TableLine::new(4, 5, 0),      // 4
        TableLine::new(5, 4, 1),      // 5...6
        TableLine::new(7, 3, 3),      // 7...14
        TableLine::new(15, 6, 1),     // 15...16
        TableLine::new(17, 6, 2),     // 17...20
        TableLine::new(21, 6, 3),     // 21...28
        TableLine::new(29, 6, 4),     // 29...44
        TableLine::new(45, 6, 5),     // 45...76
        TableLine::new(77, 7, 6),     // 77...140
        TableLine::upper(141, 7, 32), // 141...∞
    ])
});

/// Table B.14 – Standard Huffman table N (HTOOB = 0)
pub static TABLE_N: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-2, 3, 0), // -2
        TableLine::new(-1, 3, 0), // -1
        TableLine::new(0, 1, 0),  // 0
        TableLine::new(1, 3, 0),  // 1
        TableLine::new(2, 3, 0),  // 2
    ])
});

/// Table B.15 – Standard Huffman table O (HTOOB = 0)
pub static TABLE_O: LazyLock<HuffmanTable> = LazyLock::new(|| {
    HuffmanTable::build(&[
        TableLine::new(-24, 7, 4),    // -24...-9
        TableLine::new(-8, 6, 2),     // -8...-5
        TableLine::new(-4, 5, 1),     // -4...-3
        TableLine::new(-2, 4, 0),     // -2
        TableLine::new(-1, 3, 0),     // -1
        TableLine::new(0, 1, 0),      // 0
        TableLine::new(1, 3, 0),      // 1
        TableLine::new(2, 4, 0),      // 2
        TableLine::new(3, 5, 1),      // 3...4
        TableLine::new(5, 6, 2),      // 5...8
        TableLine::new(9, 7, 4),      // 9...24
        TableLine::lower(-25, 7, 32), // -∞...-25
        TableLine::upper(25, 7, 32),  // 25...∞
    ])
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to decode multiple values from a continuous bitstream.
    fn decode_all(table: &HuffmanTable, data: &[u8], expected: &[HuffmanResult]) {
        let mut reader = Reader::new(data);
        for (i, &exp) in expected.iter().enumerate() {
            let result = table.decode(&mut reader).unwrap();
            assert_eq!(result, exp, "mismatch at index {i}");
        }
    }

    #[test]
    fn test_read_custom_table_spec_example() {
        // Example from B.2: encodes a table equivalent to Table A
        let header = [
            0x42, // flags: HTOOB=0, HTPS=2, HTRS=5
            0x00, 0x00, 0x00, 0x00, // HTLOW = 0
            0x00, 0x01, 0x01, 0x10, // HTHIGH = 65808
            0x49, 0x23, 0x81, 0x80, // table lines
        ];
        let mut reader = Reader::new(&header);
        let table = HuffmanTable::read_custom(&mut reader).unwrap();

        // Test decoding same as TABLE_A
        // 0...15: prefix=0, rangelen=4
        decode_all(&table, &[0b0_0000_000], &[HuffmanResult::Value(0)]);
        decode_all(&table, &[0b0_1111_000], &[HuffmanResult::Value(15)]);
        decode_all(&table, &[0b0_0111_000], &[HuffmanResult::Value(7)]);

        // 16...271: prefix=10, rangelen=8
        decode_all(
            &table,
            &[0b10_000000, 0b00_000000],
            &[HuffmanResult::Value(16)],
        );
        decode_all(
            &table,
            &[0b10_111111, 0b11_000000],
            &[HuffmanResult::Value(271)],
        );

        // 272...65807: prefix=110, rangelen=16
        decode_all(
            &table,
            &[0b110_00000, 0b00000000, 0b0_0000000],
            &[HuffmanResult::Value(272)],
        );

        // 65808...∞: prefix=111, rangelen=32
        decode_all(
            &table,
            &[0b111_00000, 0x00, 0x00, 0x00, 0b00000_000],
            &[HuffmanResult::Value(65808)],
        );
    }
}
