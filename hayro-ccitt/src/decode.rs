use crate::bit::BitReader;
use crate::states::{
    BLACK_STATES, EOL, INVALID, MODE_STATES, Mode, State, VALUE_FLAG, VALUE_MASK, WHITE_STATES,
};
use crate::{DecodeError, Result};

impl BitReader<'_> {
    #[inline(always)]
    fn decode_run_inner(&mut self, states: &[State]) -> Result<u16> {
        let mut total: u16 = 0;
        let mut state: usize = 0;

        loop {
            let bit = self.read_bit()?;

            let transition = if bit == 0 {
                states[state].on_0
            } else {
                states[state].on_1
            };

            if transition == INVALID {
                return Err(DecodeError::InvalidCode);
            } else if transition & VALUE_FLAG != 0 {
                let len = transition & VALUE_MASK;
                total = total.checked_add(len).ok_or(DecodeError::Overflow)?;

                // For decoding black/white runs, less than 64 means we have
                // a terminating code. For mode decoding, all values are less
                // than 64 anyway, so this condition can be used for all methods.
                if len < 64 {
                    return Ok(total);
                }

                state = 0;
            } else {
                state = transition as usize;
            }
        }
    }

    #[inline(always)]
    pub(crate) fn decode_white_run(&mut self) -> Result<u16> {
        self.decode_run_inner(&WHITE_STATES)
    }

    #[inline(always)]
    pub(crate) fn decode_black_run(&mut self) -> Result<u16> {
        self.decode_run_inner(&BLACK_STATES)
    }

    #[inline(always)]
    pub(crate) fn decode_run(&mut self, is_white: bool) -> Result<u16> {
        if is_white {
            self.decode_white_run()
        } else {
            self.decode_black_run()
        }
    }

    #[inline(always)]
    pub(crate) fn decode_mode(&mut self) -> Result<Mode> {
        let mode_id = self.decode_run_inner(&MODE_STATES)?;
        Ok(match mode_id {
            0 => Mode::Pass,
            1 => Mode::Horizontal,
            2 => Mode::Vertical(0),
            3 => Mode::Vertical(1),
            4 => Mode::Vertical(2),
            5 => Mode::Vertical(3),
            6 => Mode::Vertical(-1),
            7 => Mode::Vertical(-2),
            8 => Mode::Vertical(-3),
            _ => return Err(DecodeError::InvalidCode),
        })
    }

    #[inline(always)]
    pub(crate) fn read_eol_if_available(&mut self) -> usize {
        let mut count = 0;
        while self.peak_bits(12) == Ok(EOL) {
            count += 1;
            self.read_bits(12).unwrap();
        }

        count
    }
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)]
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
        assert_eq!(reader.decode_white_run(), Ok(2));

        // Test white run length 0: code = 00110101 (8 bits)
        let data = [0b00110101];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Ok(0));

        // Test white run length 63: code = 00110100 (8 bits)
        let data = [0b00110100];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Ok(63));
    }

    // =========================================================================
    // Black terminating code tests
    // =========================================================================

    #[test]
    fn test_black_terminating_codes() {
        // Test black run length 2: code = 11 (2 bits)
        let data = [0b1100_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Ok(2));

        // Test black run length 1: code = 010 (3 bits)
        let data = [0b010_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Ok(1));

        // Test black run length 0: code = 0000110111 (10 bits)
        let data = [0b00001101, 0b11_000000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Ok(0));
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
        assert_eq!(reader.decode_white_run(), Ok(64));

        // Test white run length 128 + 5 = 133
        // Makeup 128 = 10010 (5 bits), Terminal 5 = 1100 (4 bits)
        let data = [0b10010_110, 0b0_0000000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Ok(133));
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
        assert_eq!(reader.decode_black_run(), Ok(66));
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
        assert_eq!(reader.decode_white_run(), Ok(128));

        // Test white run length 64 + 128 + 10 = 202
        // Makeup 64 = 11011 (5 bits), Makeup 128 = 10010 (5 bits), Terminal 10 = 00111 (5 bits)
        // Bits: 11011_10010_00111
        let data = [0b11011_100, 0b10_00111_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Ok(202));
    }

    #[test]
    fn test_white_three_makeup_codes() {
        // Test white run length 64 + 64 + 64 + 1 = 193
        // Makeup 64 = 11011 (5 bits) x3, Terminal 1 = 000111 (6 bits)
        // Bits: 11011_11011_11011_000111
        let data = [0b11011_110, 0b11_11011_0, 0b00111_000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Ok(193));
    }

    #[test]
    fn test_black_multiple_makeup() {
        // Test black run length 64 + 64 + 1 = 129
        // Makeup 64 = 0000001111 (10 bits) x2, Terminal 1 = 010 (3 bits)
        // Bits: 0000001111_0000001111_010
        let data = [0b00000011, 0b11_000000, 0b1111_010_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Ok(129));
    }

    // =========================================================================
    // Mode code tests
    // =========================================================================

    #[test]
    fn test_mode_codes() {
        // Vertical(0): code = 1 (1 bit)
        let data = [0b1000_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(0)));

        // Horizontal: code = 001 (3 bits)
        let data = [0b001_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Horizontal));

        // Pass: code = 0001 (4 bits)
        let data = [0b0001_0000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Pass));

        // Vertical(1): code = 011 (3 bits)
        let data = [0b011_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(1)));

        // Vertical(-1): code = 010 (3 bits)
        let data = [0b010_00000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(-1)));

        // Vertical(2): code = 000011 (6 bits)
        let data = [0b000011_00];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(2)));

        // Vertical(-2): code = 000010 (6 bits)
        let data = [0b000010_00];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(-2)));

        // Vertical(3): code = 0000011 (7 bits)
        let data = [0b0000011_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(3)));

        // Vertical(-3): code = 0000010 (7 bits)
        let data = [0b0000010_0];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Ok(Mode::Vertical(-3)));
    }

    // =========================================================================
    // Error handling tests
    // =========================================================================

    #[test]
    fn test_unexpected_eof() {
        use crate::DecodeError;

        // Empty data
        let data = [];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_white_run(), Err(DecodeError::UnexpectedEof));

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_black_run(), Err(DecodeError::UnexpectedEof));

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.decode_mode(), Err(DecodeError::UnexpectedEof));
    }
}
