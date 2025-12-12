//! CCITT Huffman decoding using state machines.
//!
//! Each state machine is a trie where each node has two transitions (bit 0 and bit 1).
//! Terminal nodes return the decoded run length.

use crate::bit::BitReader;
use log::warn;

/// Result of decoding a 2D mode code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Pass,
    Horizontal,
    Vertical0,
    VerticalR1,
    VerticalR2,
    VerticalR3,
    VerticalL1,
    VerticalL2,
    VerticalL3,
}

// State machine encoding:
// - 0x0000-0x3FFF: next state index
// - 0x8000 | value: decoded run length (value & 0x1FFF)
// - 0xFFFF: invalid/unused

const VALUE_FLAG: u16 = 0x8000;
const VALUE_MASK: u16 = 0x1FFF;
const INVALID: u16 = 0xFFFF;

/// A state in the decoding state machine.
/// Each state has two transitions: one for bit 0 and one for bit 1.
#[derive(Clone, Copy)]
struct State {
    on_0: u16,
    on_1: u16,
}

impl State {
    const fn new() -> Self {
        Self {
            on_0: INVALID,
            on_1: INVALID,
        }
    }
}

// =============================================================================
// STATE MACHINE GENERATION (compile-time)
// =============================================================================

/// Insert a single code into the state machine.
/// Returns the new number of states.
const fn insert_code(
    states: &mut [State; 512],
    num_states: usize,
    run_length: u16,
    code_length: u8,
    code: u16,
) -> usize {
    let mut num_states = num_states;
    let mut current_state: usize = 0;
    let mut i: u8 = 0;

    while i < code_length {
        let bit = (code >> (code_length - 1 - i)) & 1;
        let is_last = i == code_length - 1;

        let next = if bit == 0 {
            states[current_state].on_0
        } else {
            states[current_state].on_1
        };

        if is_last {
            // Terminal state - store the result
            let result = VALUE_FLAG | (run_length & VALUE_MASK);

            if bit == 0 {
                states[current_state].on_0 = result;
            } else {
                states[current_state].on_1 = result;
            }
        } else if next == INVALID || next >= VALUE_FLAG {
            // Need to create a new state
            let new_state = num_states;
            num_states += 1;

            if bit == 0 {
                states[current_state].on_0 = new_state as u16;
            } else {
                states[current_state].on_1 = new_state as u16;
            }
            current_state = new_state;
        } else {
            // Follow existing transition
            current_state = next as usize;
        }

        i += 1;
    }

    num_states
}

/// Build white code state machine at compile time.
const fn build_white_states() -> [State; 512] {
    let mut states: [State; 512] = [State::new(); 512];
    let mut num_states: usize = 1;

    // Insert WHITE_TERMINATING
    let mut i = 0;
    while i < WHITE_TERMINATING.len() {
        let (run_length, code_length, code) = WHITE_TERMINATING[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    // Insert WHITE_MAKEUP
    i = 0;
    while i < WHITE_MAKEUP.len() {
        let (run_length, code_length, code) = WHITE_MAKEUP[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    // Insert COMMON_MAKEUP
    i = 0;
    while i < COMMON_MAKEUP.len() {
        let (run_length, code_length, code) = COMMON_MAKEUP[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    states
}

/// Build black code state machine at compile time.
const fn build_black_states() -> [State; 512] {
    let mut states: [State; 512] = [State::new(); 512];
    let mut num_states: usize = 1;

    // Insert BLACK_TERMINATING
    let mut i = 0;
    while i < BLACK_TERMINATING.len() {
        let (run_length, code_length, code) = BLACK_TERMINATING[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    // Insert BLACK_MAKEUP
    i = 0;
    while i < BLACK_MAKEUP.len() {
        let (run_length, code_length, code) = BLACK_MAKEUP[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    // Insert COMMON_MAKEUP
    i = 0;
    while i < COMMON_MAKEUP.len() {
        let (run_length, code_length, code) = COMMON_MAKEUP[i];
        num_states = insert_code(&mut states, num_states, run_length, code_length, code);
        i += 1;
    }

    states
}

// =============================================================================
// CODE TABLES
// =============================================================================

// Format: (run_length, code_length, code)

const WHITE_TERMINATING: &[(u16, u8, u16)] = &[
    (0, 8, 0b00110101),
    (1, 6, 0b000111),
    (2, 4, 0b0111),
    (3, 4, 0b1000),
    (4, 4, 0b1011),
    (5, 4, 0b1100),
    (6, 4, 0b1110),
    (7, 4, 0b1111),
    (8, 5, 0b10011),
    (9, 5, 0b10100),
    (10, 5, 0b00111),
    (11, 5, 0b01000),
    (12, 6, 0b001000),
    (13, 6, 0b000011),
    (14, 6, 0b110100),
    (15, 6, 0b110101),
    (16, 6, 0b101010),
    (17, 6, 0b101011),
    (18, 7, 0b0100111),
    (19, 7, 0b0001100),
    (20, 7, 0b0001000),
    (21, 7, 0b0010111),
    (22, 7, 0b0000011),
    (23, 7, 0b0000100),
    (24, 7, 0b0101000),
    (25, 7, 0b0101011),
    (26, 7, 0b0010011),
    (27, 7, 0b0100100),
    (28, 7, 0b0011000),
    (29, 8, 0b00000010),
    (30, 8, 0b00000011),
    (31, 8, 0b00011010),
    (32, 8, 0b00011011),
    (33, 8, 0b00010010),
    (34, 8, 0b00010011),
    (35, 8, 0b00010100),
    (36, 8, 0b00010101),
    (37, 8, 0b00010110),
    (38, 8, 0b00010111),
    (39, 8, 0b00101000),
    (40, 8, 0b00101001),
    (41, 8, 0b00101010),
    (42, 8, 0b00101011),
    (43, 8, 0b00101100),
    (44, 8, 0b00101101),
    (45, 8, 0b00000100),
    (46, 8, 0b00000101),
    (47, 8, 0b00001010),
    (48, 8, 0b00001011),
    (49, 8, 0b01010010),
    (50, 8, 0b01010011),
    (51, 8, 0b01010100),
    (52, 8, 0b01010101),
    (53, 8, 0b00100100),
    (54, 8, 0b00100101),
    (55, 8, 0b01011000),
    (56, 8, 0b01011001),
    (57, 8, 0b01011010),
    (58, 8, 0b01011011),
    (59, 8, 0b01001010),
    (60, 8, 0b01001011),
    (61, 8, 0b00110010),
    (62, 8, 0b00110011),
    (63, 8, 0b00110100),
];

const WHITE_MAKEUP: &[(u16, u8, u16)] = &[
    (64, 5, 0b11011),
    (128, 5, 0b10010),
    (192, 6, 0b010111),
    (256, 7, 0b0110111),
    (320, 8, 0b00110110),
    (384, 8, 0b00110111),
    (448, 8, 0b01100100),
    (512, 8, 0b01100101),
    (576, 8, 0b01101000),
    (640, 8, 0b01100111),
    (704, 9, 0b011001100),
    (768, 9, 0b011001101),
    (832, 9, 0b011010010),
    (896, 9, 0b011010011),
    (960, 9, 0b011010100),
    (1024, 9, 0b011010101),
    (1088, 9, 0b011010110),
    (1152, 9, 0b011010111),
    (1216, 9, 0b011011000),
    (1280, 9, 0b011011001),
    (1344, 9, 0b011011010),
    (1408, 9, 0b011011011),
    (1472, 9, 0b010011000),
    (1536, 9, 0b010011001),
    (1600, 9, 0b010011010),
    (1664, 6, 0b011000),
    (1728, 9, 0b010011011),
];

const BLACK_TERMINATING: &[(u16, u8, u16)] = &[
    (0, 10, 0b0000110111),
    (1, 3, 0b010),
    (2, 2, 0b11),
    (3, 2, 0b10),
    (4, 3, 0b011),
    (5, 4, 0b0011),
    (6, 4, 0b0010),
    (7, 5, 0b00011),
    (8, 6, 0b000101),
    (9, 6, 0b000100),
    (10, 7, 0b0000100),
    (11, 7, 0b0000101),
    (12, 7, 0b0000111),
    (13, 8, 0b00000100),
    (14, 8, 0b00000111),
    (15, 9, 0b000011000),
    (16, 10, 0b0000010111),
    (17, 10, 0b0000011000),
    (18, 10, 0b0000001000),
    (19, 11, 0b00001100111),
    (20, 11, 0b00001101000),
    (21, 11, 0b00001101100),
    (22, 11, 0b00000110111),
    (23, 11, 0b00000101000),
    (24, 11, 0b00000010111),
    (25, 11, 0b00000011000),
    (26, 12, 0b000011001010),
    (27, 12, 0b000011001011),
    (28, 12, 0b000011001100),
    (29, 12, 0b000011001101),
    (30, 12, 0b000001101000),
    (31, 12, 0b000001101001),
    (32, 12, 0b000001101010),
    (33, 12, 0b000001101011),
    (34, 12, 0b000011010010),
    (35, 12, 0b000011010011),
    (36, 12, 0b000011010100),
    (37, 12, 0b000011010101),
    (38, 12, 0b000011010110),
    (39, 12, 0b000011010111),
    (40, 12, 0b000001101100),
    (41, 12, 0b000001101101),
    (42, 12, 0b000011011010),
    (43, 12, 0b000011011011),
    (44, 12, 0b000001010100),
    (45, 12, 0b000001010101),
    (46, 12, 0b000001010110),
    (47, 12, 0b000001010111),
    (48, 12, 0b000001100100),
    (49, 12, 0b000001100101),
    (50, 12, 0b000001010010),
    (51, 12, 0b000001010011),
    (52, 12, 0b000000100100),
    (53, 12, 0b000000110111),
    (54, 12, 0b000000111000),
    (55, 12, 0b000000100111),
    (56, 12, 0b000000101000),
    (57, 12, 0b000001011000),
    (58, 12, 0b000001011001),
    (59, 12, 0b000000101011),
    (60, 12, 0b000000101100),
    (61, 12, 0b000001011010),
    (62, 12, 0b000001100110),
    (63, 12, 0b000001100111),
];

const BLACK_MAKEUP: &[(u16, u8, u16)] = &[
    (64, 10, 0b0000001111),
    (128, 12, 0b000011001000),
    (192, 12, 0b000011001001),
    (256, 12, 0b000001011011),
    (320, 12, 0b000000110011),
    (384, 12, 0b000000110100),
    (448, 12, 0b000000110101),
    (512, 13, 0b0000001101100),
    (576, 13, 0b0000001101101),
    (640, 13, 0b0000001001010),
    (704, 13, 0b0000001001011),
    (768, 13, 0b0000001001100),
    (832, 13, 0b0000001001101),
    (896, 13, 0b0000001110010),
    (960, 13, 0b0000001110011),
    (1024, 13, 0b0000001110100),
    (1088, 13, 0b0000001110101),
    (1152, 13, 0b0000001110110),
    (1216, 13, 0b0000001110111),
    (1280, 13, 0b0000001010010),
    (1344, 13, 0b0000001010011),
    (1408, 13, 0b0000001010100),
    (1472, 13, 0b0000001010101),
    (1536, 13, 0b0000001011010),
    (1600, 13, 0b0000001011011),
    (1664, 13, 0b0000001100100),
    (1728, 13, 0b0000001100101),
];

const COMMON_MAKEUP: &[(u16, u8, u16)] = &[
    (1792, 11, 0b00000001000),
    (1856, 11, 0b00000001100),
    (1920, 11, 0b00000001101),
    (1984, 12, 0b000000010010),
    (2048, 12, 0b000000010011),
    (2112, 12, 0b000000010100),
    (2176, 12, 0b000000010101),
    (2240, 12, 0b000000010110),
    (2304, 12, 0b000000010111),
    (2368, 12, 0b000000011100),
    (2432, 12, 0b000000011101),
    (2496, 12, 0b000000011110),
    (2560, 12, 0b000000011111),
];

// Mode codes for 2D encoding
const MODE_CODES: &[(u8, u8, u8)] = &[
    // (mode_id, code_length, code)
    (0, 4, 0b0001),    // Pass
    (1, 3, 0b001),     // Horizontal
    (2, 1, 0b1),       // Vertical_0
    (3, 3, 0b011),     // Vertical_R1
    (4, 6, 0b000011),  // Vertical_R2
    (5, 7, 0b0000011), // Vertical_R3
    (6, 3, 0b010),     // Vertical_L1
    (7, 6, 0b000010),  // Vertical_L2
    (8, 7, 0b0000010), // Vertical_L3
];

// =============================================================================
// CONST STATE MACHINES
// =============================================================================

const WHITE_STATES: [State; 512] = build_white_states();

const BLACK_STATES: [State; 512] = build_black_states();

// Mode state machine is simpler - build it inline
const MODE_STATES: [State; 16] = {
    let mut states: [State; 16] = [State::new(); 16];
    let mut num_states: usize = 1;

    let mut i = 0;
    while i < MODE_CODES.len() {
        let (mode_id, code_length, code) = MODE_CODES[i];
        let mut current_state: usize = 0;

        let mut j = 0;
        while j < code_length {
            let bit = (code >> (code_length - 1 - j)) & 1;
            let is_last = j == code_length - 1;

            if is_last {
                // Store mode_id with terminal flag
                let result = VALUE_FLAG | (mode_id as u16);
                if bit == 0 {
                    states[current_state].on_0 = result;
                } else {
                    states[current_state].on_1 = result;
                }
            } else {
                let next = if bit == 0 {
                    states[current_state].on_0
                } else {
                    states[current_state].on_1
                };

                if next == INVALID || next >= VALUE_FLAG {
                    let new_state = num_states;
                    num_states += 1;

                    if bit == 0 {
                        states[current_state].on_0 = new_state as u16;
                    } else {
                        states[current_state].on_1 = new_state as u16;
                    }
                    current_state = new_state;
                } else {
                    current_state = next as usize;
                }
            }
            j += 1;
        }
        i += 1;
    }

    states
};

// =============================================================================
// DECODING FUNCTIONS (impl on BitReader)
// =============================================================================

impl BitReader<'_> {
    /// Decode a complete white run length.
    /// Handles makeup codes by accumulating until a terminating code is found.
    /// Returns `None` on EOF or invalid code (logs warning).
    pub(crate) fn decode_white_run(&mut self) -> Option<u16> {
        let mut total: u16 = 0;
        let mut state: usize = 0;

        loop {
            let bit = match self.read_bit() {
                Some(b) => b,
                None => {
                    warn!("CCITT: unexpected EOF while decoding white run");
                    return None;
                }
            };

            let transition = if bit == 0 {
                WHITE_STATES[state].on_0
            } else {
                WHITE_STATES[state].on_1
            };

            if transition == INVALID {
                warn!("CCITT: invalid white code sequence");
                return None;
            } else if transition >= VALUE_FLAG {
                let len = transition & VALUE_MASK;
                total = total.saturating_add(len);
                if len < 64 {
                    // Terminal code - we're done
                    return Some(total);
                }
                // Makeup code - reset state and continue reading
                state = 0;
            } else {
                // Continue to next state
                state = transition as usize;
            }
        }
    }

    /// Decode a complete black run length.
    /// Handles makeup codes by accumulating until a terminating code is found.
    /// Returns `None` on EOF or invalid code (logs warning).
    pub(crate) fn decode_black_run(&mut self) -> Option<u16> {
        let mut total: u16 = 0;
        let mut state: usize = 0;

        loop {
            let bit = match self.read_bit() {
                Some(b) => b,
                None => {
                    warn!("CCITT: unexpected EOF while decoding black run");
                    return None;
                }
            };

            let transition = if bit == 0 {
                BLACK_STATES[state].on_0
            } else {
                BLACK_STATES[state].on_1
            };

            if transition == INVALID {
                warn!("CCITT: invalid black code sequence");
                return None;
            } else if transition >= VALUE_FLAG {
                let len = transition & VALUE_MASK;
                total = total.saturating_add(len);
                if len < 64 {
                    // Terminal code - we're done
                    return Some(total);
                }
                // Makeup code - reset state and continue reading
                state = 0;
            } else {
                // Continue to next state
                state = transition as usize;
            }
        }
    }

    /// Decode a 2D mode code.
    /// Returns `None` on EOF or invalid code (logs warning).
    pub(crate) fn decode_mode(&mut self) -> Option<Mode> {
        let mut state: usize = 0;

        loop {
            let bit = match self.read_bit() {
                Some(b) => b,
                None => {
                    warn!("CCITT: unexpected EOF while decoding mode");
                    return None;
                }
            };

            let transition = if bit == 0 {
                MODE_STATES[state].on_0
            } else {
                MODE_STATES[state].on_1
            };

            if transition == INVALID {
                warn!("CCITT: invalid mode code sequence");
                return None;
            }

            if transition >= VALUE_FLAG {
                let mode_id = transition & VALUE_MASK;
                return Some(match mode_id {
                    0 => Mode::Pass,
                    1 => Mode::Horizontal,
                    2 => Mode::Vertical0,
                    3 => Mode::VerticalR1,
                    4 => Mode::VerticalR2,
                    5 => Mode::VerticalR3,
                    6 => Mode::VerticalL1,
                    7 => Mode::VerticalL2,
                    8 => Mode::VerticalL3,
                    _ => {
                        warn!("CCITT: invalid mode id {}", mode_id);
                        return None;
                    }
                });
            }

            state = transition as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // White terminating code tests
    // =========================================================================

    #[test]
    fn test_white_terminating_codes() {
        // Test white run length 2: code = 0111 (4 bits)
        let data = [0b0111_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(2));

        // Test white run length 0: code = 00110101 (8 bits)
        let data = [0b00110101];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(0));

        // Test white run length 63: code = 00110100 (8 bits)
        let data = [0b00110100];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(63));
    }

    // =========================================================================
    // Black terminating code tests
    // =========================================================================

    #[test]
    fn test_black_terminating_codes() {
        // Test black run length 2: code = 11 (2 bits)
        let data = [0b1100_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Some(2));

        // Test black run length 1: code = 010 (3 bits)
        let data = [0b010_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Some(1));

        // Test black run length 0: code = 0000110111 (10 bits)
        let data = [0b00001101, 0b11_000000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Some(0));
    }

    // =========================================================================
    // White makeup code tests (single makeup + terminating)
    // =========================================================================

    #[test]
    fn test_white_single_makeup() {
        // Test white run length 64 + 0 = 64
        // Makeup 64 = 11011 (5 bits), Terminal 0 = 00110101 (8 bits)
        let data = [0b11011_001, 0b10101_000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(64));

        // Test white run length 128 + 5 = 133
        // Makeup 128 = 10010 (5 bits), Terminal 5 = 1100 (4 bits)
        let data = [0b10010_110, 0b0_0000000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(133));
    }

    // =========================================================================
    // Black makeup code tests (single makeup + terminating)
    // =========================================================================

    #[test]
    fn test_black_single_makeup() {
        // Test black run length 64 + 2 = 66
        // Makeup 64 = 0000001111 (10 bits), Terminal 2 = 11 (2 bits)
        let data = [0b00000011, 0b11_11_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Some(66));
    }

    // =========================================================================
    // Multiple makeup codes tests
    // =========================================================================

    #[test]
    fn test_white_multiple_makeup() {
        // Test white run length 64 + 64 + 0 = 128
        // Makeup 64 = 11011 (5 bits), Makeup 64 = 11011 (5 bits), Terminal 0 = 00110101 (8 bits)
        // Bits: 11011_11011_00110101
        let data = [0b11011_110, 0b11_001101, 0b01_000000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(128));

        // Test white run length 64 + 128 + 10 = 202
        // Makeup 64 = 11011 (5 bits), Makeup 128 = 10010 (5 bits), Terminal 10 = 00111 (5 bits)
        // Bits: 11011_10010_00111
        let data = [0b11011_100, 0b10_00111_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(202));
    }

    #[test]
    fn test_white_three_makeup_codes() {
        // Test white run length 64 + 64 + 64 + 1 = 193
        // Makeup 64 = 11011 (5 bits) x3, Terminal 1 = 000111 (6 bits)
        // Bits: 11011_11011_11011_000111
        let data = [0b11011_110, 0b11_11011_0, 0b00111_000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Some(193));
    }

    #[test]
    fn test_black_multiple_makeup() {
        // Test black run length 64 + 64 + 1 = 129
        // Makeup 64 = 0000001111 (10 bits) x2, Terminal 1 = 010 (3 bits)
        // Bits: 0000001111_0000001111_010
        let data = [0b00000011, 0b11_000000, 0b1111_010_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Some(129));
    }

    // =========================================================================
    // Mode code tests
    // =========================================================================

    #[test]
    fn test_mode_codes() {
        // Vertical_0: code = 1 (1 bit)
        let data = [0b1000_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::Vertical0));

        // Horizontal: code = 001 (3 bits)
        let data = [0b001_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::Horizontal));

        // Pass: code = 0001 (4 bits)
        let data = [0b0001_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::Pass));

        // Vertical_R1: code = 011 (3 bits)
        let data = [0b011_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalR1));

        // Vertical_L1: code = 010 (3 bits)
        let data = [0b010_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalL1));

        // Vertical_R2: code = 000011 (6 bits)
        let data = [0b000011_00];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalR2));

        // Vertical_L2: code = 000010 (6 bits)
        let data = [0b000010_00];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalL2));

        // Vertical_R3: code = 0000011 (7 bits)
        let data = [0b0000011_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalR3));

        // Vertical_L3: code = 0000010 (7 bits)
        let data = [0b0000010_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Some(Mode::VerticalL3));
    }

    // =========================================================================
    // Error handling tests
    // =========================================================================

    #[test]
    fn test_unexpected_eof() {
        // Empty data
        let data = [];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), None);

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), None);

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), None);
    }
}
