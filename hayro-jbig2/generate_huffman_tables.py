#!/usr/bin/env python3
"""
Generate precomputed Huffman tables for JBIG2 decoding.

This script implements the Huffman table building algorithm from JBIG2 Annex B.3
and outputs Rust code with the precomputed tree structure.
"""

from dataclasses import dataclass
from typing import Optional


@dataclass
class TableLine:
    """A table line definition used to build the Huffman tree."""
    range_low: int
    prefix_length: int
    range_length: int
    is_lower: bool = False
    is_out_of_band: bool = False

    @staticmethod
    def new(range_low: int, prefix_length: int, range_length: int) -> "TableLine":
        return TableLine(range_low, prefix_length, range_length)

    @staticmethod
    def lower(range_high: int, prefix_length: int, range_length: int) -> "TableLine":
        return TableLine(range_high, prefix_length, range_length, is_lower=True)

    @staticmethod
    def upper(range_low: int, prefix_length: int, range_length: int) -> "TableLine":
        return TableLine(range_low, prefix_length, range_length)

    @staticmethod
    def oob(prefix_length: int) -> "TableLine":
        return TableLine(0, prefix_length, 0, is_out_of_band=True)


@dataclass
class LeafData:
    """Information stored at a leaf node."""
    range_low: int
    range_length: int
    is_lower: bool
    is_out_of_band: bool


@dataclass
class HuffmanNode:
    """A node in the Huffman tree."""
    # For intermediate nodes
    zero: Optional[int] = None
    one: Optional[int] = None
    # For leaf nodes
    leaf: Optional[LeafData] = None

    def is_intermediate(self) -> bool:
        return self.leaf is None


def build_huffman_table(lines: list[TableLine]) -> list[HuffmanNode]:
    """
    Build a Huffman table from table line definitions.
    Implements the algorithm from JBIG2 Annex B.3.
    """
    line_count = len(lines)

    # Step 1: Find maximum prefix length and build histogram
    max_prefix_length = max(line.prefix_length for line in lines) if lines else 0
    length_counts = [0] * (max_prefix_length + 1)
    for line in lines:
        length_counts[line.prefix_length] += 1

    # Step 2-3: Compute first code per length and assign codes
    first_code_per_length = [0] * (max_prefix_length + 1)
    assigned_codes = [0] * line_count
    length_counts[0] = 0

    for current_length in range(1, max_prefix_length + 1):
        first_code_per_length[current_length] = (
            first_code_per_length[current_length - 1] + length_counts[current_length - 1]
        ) * 2
        current_code = first_code_per_length[current_length]

        for line_index in range(line_count):
            if lines[line_index].prefix_length == current_length:
                assigned_codes[line_index] = current_code
                current_code += 1

    # Build tree from assigned codes
    nodes: list[HuffmanNode] = [HuffmanNode()]  # Root node

    for i, line in enumerate(lines):
        if line.prefix_length == 0:
            continue

        insert_code(
            nodes,
            0,  # root index
            assigned_codes[i],
            line.prefix_length,
            line.range_low,
            line.range_length,
            line.is_lower,
            line.is_out_of_band,
        )

    return nodes


def insert_code(
    nodes: list[HuffmanNode],
    node_index: int,
    code: int,
    prefix_length: int,
    range_low: int,
    range_length: int,
    is_lower: bool,
    is_out_of_band: bool,
) -> None:
    """Insert a code into the Huffman tree."""
    if prefix_length == 0:
        # We've consumed all bits, this should be a leaf
        nodes[node_index] = HuffmanNode(
            leaf=LeafData(range_low, range_length, is_lower, is_out_of_band)
        )
        return

    # Get the next bit (MSB first)
    bit = (code >> (prefix_length - 1)) & 1
    remaining_code = code & ((1 << (prefix_length - 1)) - 1)

    node = nodes[node_index]
    assert node.is_intermediate(), "Attempted to insert code into leaf node"

    if bit == 0:
        if node.zero is None:
            new_idx = len(nodes)
            nodes.append(HuffmanNode())
            nodes[node_index].zero = new_idx
            child_index = new_idx
        else:
            child_index = node.zero
    else:
        if node.one is None:
            new_idx = len(nodes)
            nodes.append(HuffmanNode())
            nodes[node_index].one = new_idx
            child_index = new_idx
        else:
            child_index = node.one

    insert_code(
        nodes,
        child_index,
        remaining_code,
        prefix_length - 1,
        range_low,
        range_length,
        is_lower,
        is_out_of_band,
    )


def generate_rust_node(node: HuffmanNode) -> str:
    """Generate Rust code for a single node."""
    if node.is_intermediate():
        zero = f"Some(nz({node.zero}))" if node.zero is not None else "None"
        one = f"Some(nz({node.one}))" if node.one is not None else "None"
        return f"N::Intermediate {{ zero: {zero}, one: {one} }}"
    else:
        leaf = node.leaf
        assert leaf is not None
        return (
            f"N::Leaf(L {{ range_low: {leaf.range_low}, range_length: {leaf.range_length}, "
            f"is_lower: {str(leaf.is_lower).lower()}, is_out_of_band: {str(leaf.is_out_of_band).lower()} }})"
        )


def generate_rust_table(name: str, nodes: list[HuffmanNode]) -> str:
    """Generate Rust code for a complete table."""
    node_strs = [generate_rust_node(node) for node in nodes]

    # Pad to INLINE_TABLE_SIZE (43) nodes with Empty variants
    while len(node_strs) < 43:
        node_strs.append("N::Empty")

    nodes_str = ",\n        ".join(node_strs)
    return f"""#[rustfmt::skip]
const {name}: [N; INLINE_TABLE_SIZE] = [
        {nodes_str},
];"""


# Standard Huffman tables from JBIG2 Annex B
STANDARD_TABLES = {
    "TABLE_A": [
        TableLine.new(0, 1, 4),        # 0...15
        TableLine.new(16, 2, 8),       # 16...271
        TableLine.new(272, 3, 16),     # 272...65807
        TableLine.upper(65808, 3, 32), # 65808...inf
    ],
    "TABLE_B": [
        TableLine.new(0, 1, 0),     # 0
        TableLine.new(1, 2, 0),     # 1
        TableLine.new(2, 3, 0),     # 2
        TableLine.new(3, 4, 3),     # 3...10
        TableLine.new(11, 5, 6),    # 11...74
        TableLine.upper(75, 6, 32), # 75...inf
        TableLine.oob(6),           # OOB
    ],
    "TABLE_C": [
        TableLine.new(-256, 8, 8),    # -256...-1
        TableLine.new(0, 1, 0),       # 0
        TableLine.new(1, 2, 0),       # 1
        TableLine.new(2, 3, 0),       # 2
        TableLine.new(3, 4, 3),       # 3...10
        TableLine.new(11, 5, 6),      # 11...74
        TableLine.lower(-257, 8, 32), # -inf...-257
        TableLine.upper(75, 7, 32),   # 75...inf
        TableLine.oob(6),             # OOB
    ],
    "TABLE_D": [
        TableLine.new(1, 1, 0),     # 1
        TableLine.new(2, 2, 0),     # 2
        TableLine.new(3, 3, 0),     # 3
        TableLine.new(4, 4, 3),     # 4...11
        TableLine.new(12, 5, 6),    # 12...75
        TableLine.upper(76, 5, 32), # 76...inf
    ],
    "TABLE_E": [
        TableLine.new(-255, 7, 8),    # -255...0
        TableLine.new(1, 1, 0),       # 1
        TableLine.new(2, 2, 0),       # 2
        TableLine.new(3, 3, 0),       # 3
        TableLine.new(4, 4, 3),       # 4...11
        TableLine.new(12, 5, 6),      # 12...75
        TableLine.lower(-256, 7, 32), # -inf...-256
        TableLine.upper(76, 6, 32),   # 76...inf
    ],
    "TABLE_F": [
        TableLine.new(-2048, 5, 10),   # -2048...-1025
        TableLine.new(-1024, 4, 9),    # -1024...-513
        TableLine.new(-512, 4, 8),     # -512...-257
        TableLine.new(-256, 4, 7),     # -256...-129
        TableLine.new(-128, 5, 6),     # -128...-65
        TableLine.new(-64, 5, 5),      # -64...-33
        TableLine.new(-32, 4, 5),      # -32...-1
        TableLine.new(0, 2, 7),        # 0...127
        TableLine.new(128, 3, 7),      # 128...255
        TableLine.new(256, 3, 8),      # 256...511
        TableLine.new(512, 4, 9),      # 512...1023
        TableLine.new(1024, 4, 10),    # 1024...2047
        TableLine.lower(-2049, 6, 32), # -inf...-2049
        TableLine.upper(2048, 6, 32),  # 2048...inf
    ],
    "TABLE_G": [
        TableLine.new(-1024, 4, 9),    # -1024...-513
        TableLine.new(-512, 3, 8),     # -512...-257
        TableLine.new(-256, 4, 7),     # -256...-129
        TableLine.new(-128, 5, 6),     # -128...-65
        TableLine.new(-64, 5, 5),      # -64...-33
        TableLine.new(-32, 4, 5),      # -32...-1
        TableLine.new(0, 4, 5),        # 0...31
        TableLine.new(32, 5, 5),       # 32...63
        TableLine.new(64, 5, 6),       # 64...127
        TableLine.new(128, 4, 7),      # 128...255
        TableLine.new(256, 3, 8),      # 256...511
        TableLine.new(512, 3, 9),      # 512...1023
        TableLine.new(1024, 3, 10),    # 1024...2047
        TableLine.lower(-1025, 5, 32), # -inf...-1025
        TableLine.upper(2048, 5, 32),  # 2048...inf
    ],
    "TABLE_H": [
        TableLine.new(-15, 8, 3),     # -15...-8
        TableLine.new(-7, 9, 1),      # -7...-6
        TableLine.new(-5, 8, 1),      # -5...-4
        TableLine.new(-3, 9, 0),      # -3
        TableLine.new(-2, 7, 0),      # -2
        TableLine.new(-1, 4, 0),      # -1
        TableLine.new(0, 2, 1),       # 0...1
        TableLine.new(2, 5, 0),       # 2
        TableLine.new(3, 6, 0),       # 3
        TableLine.new(4, 3, 4),       # 4...19
        TableLine.new(20, 6, 1),      # 20...21
        TableLine.new(22, 4, 4),      # 22...37
        TableLine.new(38, 4, 5),      # 38...69
        TableLine.new(70, 5, 6),      # 70...133
        TableLine.new(134, 5, 7),     # 134...261
        TableLine.new(262, 6, 7),     # 262...389
        TableLine.new(390, 7, 8),     # 390...645
        TableLine.new(646, 6, 10),    # 646...1669
        TableLine.lower(-16, 9, 32),  # -inf...-16
        TableLine.upper(1670, 9, 32), # 1670...inf
        TableLine.oob(2),             # OOB
    ],
    "TABLE_I": [
        TableLine.new(-31, 8, 4),     # -31...-16
        TableLine.new(-15, 9, 2),     # -15...-12
        TableLine.new(-11, 8, 2),     # -11...-8
        TableLine.new(-7, 9, 1),      # -7...-6
        TableLine.new(-5, 7, 1),      # -5...-4
        TableLine.new(-3, 4, 1),      # -3...-2
        TableLine.new(-1, 3, 1),      # -1...0
        TableLine.new(1, 3, 1),       # 1...2
        TableLine.new(3, 5, 1),       # 3...4
        TableLine.new(5, 6, 1),       # 5...6
        TableLine.new(7, 3, 5),       # 7...38
        TableLine.new(39, 6, 2),      # 39...42
        TableLine.new(43, 4, 5),      # 43...74
        TableLine.new(75, 4, 6),      # 75...138
        TableLine.new(139, 5, 7),     # 139...266
        TableLine.new(267, 5, 8),     # 267...522
        TableLine.new(523, 6, 8),     # 523...778
        TableLine.new(779, 7, 9),     # 779...1290
        TableLine.new(1291, 6, 11),   # 1291...3338
        TableLine.lower(-32, 9, 32),  # -inf...-32
        TableLine.upper(3339, 9, 32), # 3339...inf
        TableLine.oob(2),             # OOB
    ],
    "TABLE_J": [
        TableLine.new(-21, 7, 4),     # -21...-6
        TableLine.new(-5, 8, 0),      # -5
        TableLine.new(-4, 7, 0),      # -4
        TableLine.new(-3, 5, 0),      # -3
        TableLine.new(-2, 2, 2),      # -2...1
        TableLine.new(2, 5, 0),       # 2
        TableLine.new(3, 6, 0),       # 3
        TableLine.new(4, 7, 0),       # 4
        TableLine.new(5, 8, 0),       # 5
        TableLine.new(6, 2, 6),       # 6...69
        TableLine.new(70, 5, 5),      # 70...101
        TableLine.new(102, 6, 5),     # 102...133
        TableLine.new(134, 6, 6),     # 134...197
        TableLine.new(198, 6, 7),     # 198...325
        TableLine.new(326, 6, 8),     # 326...581
        TableLine.new(582, 6, 9),     # 582...1093
        TableLine.new(1094, 6, 10),   # 1094...2117
        TableLine.new(2118, 7, 11),   # 2118...4165
        TableLine.lower(-22, 8, 32),  # -inf...-22
        TableLine.upper(4166, 8, 32), # 4166...inf
        TableLine.oob(2),             # OOB
    ],
    "TABLE_K": [
        TableLine.new(1, 1, 0),      # 1
        TableLine.new(2, 2, 1),      # 2...3
        TableLine.new(4, 4, 0),      # 4
        TableLine.new(5, 4, 1),      # 5...6
        TableLine.new(7, 5, 1),      # 7...8
        TableLine.new(9, 5, 2),      # 9...12
        TableLine.new(13, 6, 2),     # 13...16
        TableLine.new(17, 7, 2),     # 17...20
        TableLine.new(21, 7, 3),     # 21...28
        TableLine.new(29, 7, 4),     # 29...44
        TableLine.new(45, 7, 5),     # 45...76
        TableLine.new(77, 7, 6),     # 77...140
        TableLine.upper(141, 7, 32), # 141...inf
    ],
    "TABLE_L": [
        TableLine.new(1, 1, 0),     # 1
        TableLine.new(2, 2, 0),     # 2
        TableLine.new(3, 3, 1),     # 3...4
        TableLine.new(5, 5, 0),     # 5
        TableLine.new(6, 5, 1),     # 6...7
        TableLine.new(8, 6, 1),     # 8...9
        TableLine.new(10, 7, 0),    # 10
        TableLine.new(11, 7, 1),    # 11...12
        TableLine.new(13, 7, 2),    # 13...16
        TableLine.new(17, 7, 3),    # 17...24
        TableLine.new(25, 7, 4),    # 25...40
        TableLine.new(41, 8, 5),    # 41...72
        TableLine.upper(73, 8, 32), # 73...inf
    ],
    "TABLE_M": [
        TableLine.new(1, 1, 0),      # 1
        TableLine.new(2, 3, 0),      # 2
        TableLine.new(3, 4, 0),      # 3
        TableLine.new(4, 5, 0),      # 4
        TableLine.new(5, 4, 1),      # 5...6
        TableLine.new(7, 3, 3),      # 7...14
        TableLine.new(15, 6, 1),     # 15...16
        TableLine.new(17, 6, 2),     # 17...20
        TableLine.new(21, 6, 3),     # 21...28
        TableLine.new(29, 6, 4),     # 29...44
        TableLine.new(45, 6, 5),     # 45...76
        TableLine.new(77, 7, 6),     # 77...140
        TableLine.upper(141, 7, 32), # 141...inf
    ],
    "TABLE_N": [
        TableLine.new(-2, 3, 0), # -2
        TableLine.new(-1, 3, 0), # -1
        TableLine.new(0, 1, 0),  # 0
        TableLine.new(1, 3, 0),  # 1
        TableLine.new(2, 3, 0),  # 2
    ],
    "TABLE_O": [
        TableLine.new(-24, 7, 4),    # -24...-9
        TableLine.new(-8, 6, 2),     # -8...-5
        TableLine.new(-4, 5, 1),     # -4...-3
        TableLine.new(-2, 4, 0),     # -2
        TableLine.new(-1, 3, 0),     # -1
        TableLine.new(0, 1, 0),      # 0
        TableLine.new(1, 3, 0),      # 1
        TableLine.new(2, 4, 0),      # 2
        TableLine.new(3, 5, 1),      # 3...4
        TableLine.new(5, 6, 2),      # 5...8
        TableLine.new(9, 7, 4),      # 9...24
        TableLine.lower(-25, 7, 32), # -inf...-25
        TableLine.upper(25, 7, 32),  # 25...inf
    ],
}


def main():
    output = []
    output.append("// Auto-generated Huffman tables. Do not edit manually.")
    output.append("// Generated by generate_huffman_tables.py")
    output.append("")
    output.append("type N = HuffmanNode;")
    output.append("type L = LeafData;")
    output.append("")
    output.append("const fn nz(n: u32) -> NonZeroU32 { NonZeroU32::new(n).unwrap() }")
    output.append("")

    # Map table names to their B.X numbers
    table_numbers = {
        "TABLE_A": 1, "TABLE_B": 2, "TABLE_C": 3, "TABLE_D": 4,
        "TABLE_E": 5, "TABLE_F": 6, "TABLE_G": 7, "TABLE_H": 8,
        "TABLE_I": 9, "TABLE_J": 10, "TABLE_K": 11, "TABLE_L": 12,
        "TABLE_M": 13, "TABLE_N": 14, "TABLE_O": 15,
    }

    for name, lines in STANDARD_TABLES.items():
        nodes = build_huffman_table(lines)
        print(f"{name}: {len(nodes)} nodes")
        table_num = table_numbers[name]
        output.append(f"/// Standard Huffman table {name[-1]} (Table B.{table_num}).")
        output.append(generate_rust_table(name, nodes))
        output.append("")

    # Write output
    output_path = "src/huffman_tables_generated.rs"
    with open(output_path, "w") as f:
        f.write("\n".join(output))
    print(f"Generated {output_path}")


if __name__ == "__main__":
    main()
