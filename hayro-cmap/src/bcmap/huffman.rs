use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use super::reader::Reader;

#[derive(Debug, Clone, Copy)]
pub(super) enum HuffmanNode {
    /// Intermediate node with optional children.
    Intermediate {
        /// Index of the zero-child.
        zero: Option<NonZeroU32>,
        /// Index of the one-child.
        one: Option<NonZeroU32>,
    },
    /// Leaf node holding a decoded symbol value.
    Leaf(u32),
}

/// A Huffman table built from canonical codes.
#[derive(Debug)]
pub(super) struct HuffmanTable {
    nodes: Vec<HuffmanNode>,
}

impl HuffmanTable {
    /// Decode a single symbol from the bit reader.
    pub(super) fn decode(&self, reader: &mut Reader<'_>) -> Option<u32> {
        let mut idx = 0_u32;

        loop {
            match self.nodes[idx as usize] {
                HuffmanNode::Intermediate { zero, one } => {
                    let bit = reader.read_bit()?;
                    let child = if bit == 0 { zero } else { one };
                    idx = child?.get();
                }
                HuffmanNode::Leaf(symbol) => return Some(symbol),
            }
        }
    }

    fn insert_code(
        nodes: &mut Vec<HuffmanNode>,
        node_index: u32,
        code: u32,
        length: u8,
        symbol: u32,
    ) {
        if length == 0 {
            nodes[node_index as usize] = HuffmanNode::Leaf(symbol);

            return;
        }

        let bit = (code >> (length - 1)) & 1;
        let remaining = code & ((1 << (length - 1)) - 1);

        let child_index = match &nodes[node_index as usize] {
            HuffmanNode::Intermediate { zero, one } => {
                let existing = if bit == 0 { *zero } else { *one };
                match existing {
                    Some(idx) => idx,
                    None => {
                        let new_idx = NonZeroU32::new(nodes.len() as u32).unwrap();
                        nodes.push(HuffmanNode::Intermediate {
                            zero: None,
                            one: None,
                        });

                        match &mut nodes[node_index as usize] {
                            HuffmanNode::Intermediate { zero, one } => {
                                if bit == 0 {
                                    *zero = Some(new_idx);
                                } else {
                                    *one = Some(new_idx);
                                }
                            }
                            _ => unreachable!(),
                        }
                        new_idx
                    }
                }
            }
            _ => return,
        };

        Self::insert_code(nodes, child_index.get(), remaining, length - 1, symbol);
    }

    fn build(code_lengths: &[u8], symbols: &[u32]) -> Self {
        debug_assert_eq!(code_lengths.len(), symbols.len());

        if symbols.is_empty() {
            return Self {
                nodes: vec![HuffmanNode::Intermediate {
                    zero: None,
                    one: None,
                }],
            };
        }

        let max_length = *code_lengths.iter().max().unwrap_or(&0) as usize;

        let mut codes = vec![0_u32; symbols.len()];
        let mut code = 0_u32;

        for length in 1..=max_length {
            for (i, &cl) in code_lengths.iter().enumerate() {
                if cl as usize == length {
                    codes[i] = code;
                    code += 1;
                }
            }
            code <<= 1;
        }

        let mut nodes = vec![HuffmanNode::Intermediate {
            zero: None,
            one: None,
        }];

        for (i, &symbol) in symbols.iter().enumerate() {
            if code_lengths[i] == 0 {
                continue;
            }

            Self::insert_code(&mut nodes, 0, codes[i], code_lengths[i], symbol);
        }

        Self { nodes }
    }
}

pub(super) fn decode_tables(data: &[u8]) -> Option<(HuffmanTable, HuffmanTable)> {
    let mut reader = Reader::new(data);
    let delta = decode_single_table(&mut reader, |r| r.read_u32())?;
    let count = decode_single_table(&mut reader, |r| r.read_u8().map(u32::from))?;

    Some((delta, count))
}

fn decode_single_table(
    reader: &mut Reader<'_>,
    read_sym: fn(&mut Reader<'_>) -> Option<u32>,
) -> Option<HuffmanTable> {
    let n_symbols = reader.read_u16()? as usize;

    if n_symbols == 0 {
        let _max_len = reader.read_u8()?;

        return Some(HuffmanTable {
            nodes: vec![HuffmanNode::Intermediate {
                zero: None,
                one: None,
            }],
        });
    }

    let max_code_length = reader.read_u8()? as usize;

    let mut counts = Vec::with_capacity(max_code_length);
    for _ in 0..max_code_length {
        counts.push(reader.read_u16()?);
    }

    let mut code_lengths = Vec::with_capacity(n_symbols);
    let mut symbols = Vec::with_capacity(n_symbols);

    for (length_idx, &count) in counts.iter().enumerate() {
        let code_len = (length_idx + 1) as u8;

        for _ in 0..count {
            symbols.push(read_sym(reader)?);
            code_lengths.push(code_len);
        }
    }

    Some(HuffmanTable::build(&code_lengths, &symbols))
}
