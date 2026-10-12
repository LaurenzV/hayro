use crate::bit_reader::BitReader;
use crate::state_machine::{BLACK_LOOKUP, LOOKUP_BITS, LOOKUP_INVALID, WHITE_LOOKUP};
use crate::{Color, DecodeError, Result};

/// 2D coding modes (T.4 Section 4.2.1.3.2, T.6 Section 2.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Pass mode (T.4 Section 4.2.1.3.2a, T.6 Section 2.2.3.1).
    Pass,
    /// Horizontal mode (T.4 Section 4.2.1.3.2c, T.6 Section 2.2.3.3).
    Horizontal,
    /// Vertical mode with offset (T.4 Section 4.2.1.3.2b, T.6 Section 2.2.3.2).
    Vertical(i8),
}

impl BitReader<'_> {
    /// Decode a run length using the given lookup table (T.4 Section 4.1.1, T.6 Section 2.2.4).
    ///
    /// Run lengths 0-63 use terminating codes.
    /// Run lengths 64+ use one or more make-up codes followed by a terminating code.
    #[inline(always)]
    fn decode_run_inner(&mut self, lookup: &[u16; 1 << LOOKUP_BITS]) -> Result<u32> {
        let mut total: u32 = 0;

        loop {
            let entry = lookup[(self.peek_u32() >> (32 - LOOKUP_BITS)) as usize];
            self.consume((entry >> 12) as usize)?;

            let len = entry & 0xFFF;

            if len == LOOKUP_INVALID {
                return Err(DecodeError::InvalidCode);
            }

            total = total.checked_add(len as u32).ok_or(DecodeError::Overflow)?;

            // For decoding black/white runs, less than 64 means we have
            // a terminating code.
            if len < 64 {
                return Ok(total);
            }
        }
    }

    /// Decode a white run length.
    #[inline(always)]
    fn decode_white_run(&mut self) -> Result<u32> {
        self.decode_run_inner(&WHITE_LOOKUP)
            // See 0506179.pdf. We are lenient and check whether perhaps
            // the opposite color works.
            .or_else(|_| self.decode_run_inner(&BLACK_LOOKUP))
    }

    /// Decode a black run length.
    #[inline(always)]
    fn decode_black_run(&mut self) -> Result<u32> {
        self.decode_run_inner(&BLACK_LOOKUP)
            // See 0506179.pdf. We are lenient and check whether perhaps
            // the opposite color works.
            .or_else(|_| self.decode_run_inner(&WHITE_LOOKUP))
    }

    /// Decode a run length for the specified color.
    #[inline(always)]
    pub(crate) fn decode_run(&mut self, color: Color) -> Result<u32> {
        match color {
            Color::White => self.decode_white_run(),
            Color::Black => self.decode_black_run(),
        }
    }

    /// Decode a 2D mode code.
    #[inline(always)]
    pub(crate) fn decode_mode(&mut self) -> Result<Mode> {
        if self.read_bit()? == 1 {
            return Ok(Mode::Vertical(0));
        }

        match self.read_bits(2)? {
            0b01 => return Ok(Mode::Horizontal),
            0b11 => return Ok(Mode::Vertical(1)),
            0b10 => return Ok(Mode::Vertical(-1)),
            0b00 => {}
            _ => unreachable!(),
        }

        if self.read_bit()? == 1 {
            return Ok(Mode::Pass);
        }

        if self.read_bit()? == 1 {
            return Ok(if self.read_bit()? == 1 {
                Mode::Vertical(2)
            } else {
                Mode::Vertical(-2)
            });
        }

        if self.read_bit()? == 0 {
            return Err(DecodeError::InvalidCode);
        }

        Ok(if self.read_bit()? == 1 {
            Mode::Vertical(3)
        } else {
            Mode::Vertical(-3)
        })
    }

    /// Read EOL (End-of-Line) codes if present (T.4 Section 4.1.2).
    ///
    /// EOL is defined as `000000000001` (11 zeros followed by a 1).
    /// Fill bits (T.4 Section 4.1.3) may precede the EOL as a variable-length
    /// string of zeros.
    pub(crate) fn read_eol_if_available(&mut self) -> usize {
        let mut count = 0;

        while let (zeros, Some(after)) = self.zero_run() {
            if zeros < 11 {
                break;
            }

            *self = after;
            count += 1;
        }

        count
    }

    /// Advance to the start of the next EOL (including any fill preceding it),
    /// without consuming it. Returns `false` and doesn't advance if there is
    /// none.
    pub(crate) fn skip_to_eol(&mut self) -> bool {
        let mut reader = self.clone();

        while let (zeros, Some(after)) = reader.zero_run() {
            if zeros >= 11 {
                *self = reader;
                return true;
            }

            reader = after;
        }

        false
    }

    /// Count the zeros at the current position and return a reader positioned
    /// after the 1 that terminates them, if there is one.
    fn zero_run(&self) -> (usize, Option<Self>) {
        let mut reader = self.clone();
        let mut zeros = 0;

        loop {
            match reader.read_bit() {
                Ok(0) => zeros += 1,
                Ok(_) => return (zeros, Some(reader)),
                Err(_) => return (zeros, None),
            }
        }
    }
}
