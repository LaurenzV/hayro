use crate::bit_reader::BitReader;
use crate::context::Color;
use crate::tables::{BLACK_LOOKUP, LOOKUP_BITS, LOOKUP_INVALID, WHITE_LOOKUP};
use crate::{DecodeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Pass,
    Horizontal,
    Vertical(i8),
}

impl BitReader<'_> {
    #[inline(always)]
    pub(crate) fn decode_run(&mut self, color: Color) -> Result<u32> {
        match color {
            Color::White => self.decode_run_inner(&WHITE_LOOKUP),
            Color::Black => self.decode_run_inner(&BLACK_LOOKUP),
        }
    }

    #[inline(always)]
    pub(crate) fn decode_mode(&mut self) -> Result<Mode> {
        let bits = self.peek_u32();

        if bits >> 31 == 1 {
            self.consume(1)?;
            return Ok(Mode::Vertical(0));
        }

        let zeros = bits.leading_zeros();
        let offset = if bits.checked_shl(zeros + 1).unwrap_or(0) >> 31 == 1 {
            1
        } else {
            -1
        };

        let (mode, len) = match zeros {
            1 => (Mode::Vertical(offset), 3),
            2 => (Mode::Horizontal, 3),
            3 => (Mode::Pass, 4),
            4 => (Mode::Vertical(2 * offset), 6),
            5 => (Mode::Vertical(3 * offset), 7),
            _ => {
                self.consume(6)?;
                return Err(DecodeError::InvalidCode);
            }
        };

        self.consume(len)?;

        Ok(mode)
    }

    pub(crate) fn read_eol_if_available(&mut self) -> usize {
        self.read_eols(usize::MAX)
    }

    pub(crate) fn read_eols(&mut self, max: usize) -> usize {
        let mut count = 0;

        while count < max
            && let (zeros, Some(after)) = self.zero_run()
        {
            if zeros < 11 {
                break;
            }

            *self = after;
            count += 1;
        }

        count
    }

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

            if len < 64 {
                return Ok(total);
            }
        }
    }

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
