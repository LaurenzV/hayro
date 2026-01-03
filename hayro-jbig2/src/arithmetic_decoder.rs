/// The arithmetic decoder, as described in Annex E.
pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying encoded data.
    data: &'a [u8],
    /// `C` - The `c_high` and `c_low` registers.
    code_register: u32,
    /// `A` - The probability interval register.
    interval: u32,
    /// `BP` - A pointer to the compressed data.
    byte_offset: u32,
    /// `CT` - The count of available bits in the current byte.
    bits_available: u32,
}

impl<'a> ArithmeticDecoder<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            code_register: 0,
            interval: 0,
            byte_offset: 0,
            bits_available: 0,
        };

        decoder.initialize();

        decoder
    }

    /// Read the next bit using the given context.
    #[inline(always)]
    pub(crate) fn decode(&mut self, context: &mut Context) -> u32 {
        self.decode_internal(context)
    }

    /// The DECODE procedure (E.3.2, Figure G.2).
    #[inline(always)]
    fn decode_internal(&mut self, context: &mut Context) -> u32 {
        let qe_entry = &QE_TABLE[context.state_index as usize];

        // Figure G.2: "A = A - Qe(I(CX))"
        self.interval -= qe_entry.probability;

        // `D` - The decoded binary decision.
        let decoded_bit;

        // Figure G.2: "Chigh < A?"
        if (self.code_register >> 16) < self.interval {
            // Figure G.2: "A AND 0x8000 = 0?"
            if self.interval & 0x8000 == 0 {
                // Figure G.2: "D = MPS_EXCHANGE; RENORMD"
                decoded_bit = self.exchange_mps(context, qe_entry);
                self.renormalize();
            } else {
                // Figure G.2: "D = MPS(CX)"
                decoded_bit = context.mps;
            }
        } else {
            // Figure G.2: "Chigh = Chigh - A; D = LPS_EXCHANGE; RENORMD"
            self.code_register -= self.interval << 16;

            decoded_bit = self.exchange_lps(context, qe_entry);
            self.renormalize();
        }

        decoded_bit
    }

    /// The INITDEC procedure (E.3.5, Figure G.1).
    #[inline(always)]
    fn initialize(&mut self) {
        // Figure G.1: "C = (B XOR 0xFF) << 16"
        self.code_register = ((self.current_byte() as u32) ^ 0xff) << 16;

        // Figure G.1: "BYTEIN"
        self.read_byte();

        // Figure G.1: "C = C << 7; CT = CT - 7; A = 0x8000"
        self.code_register <<= 7;
        self.bits_available -= 7;
        self.interval = 0x8000;
    }

    /// The BYTEIN procedure (E.3.4, Figure G.3).
    #[inline(always)]
    fn read_byte(&mut self) {
        // Figure G.3: "B = 0xFF?"
        if self.current_byte() == 0xff {
            // `B1` - The next byte.
            let next_byte = self.next_byte();

            // Figure G.3: "B1 > 0x8F?"
            if next_byte > 0x8f {
                // Figure G.3: "CT = 8"
                self.bits_available = 8;
            } else {
                // Figure G.3: "BP = BP + 1; C = C + 0xFE00 - (B << 9); CT = 7"
                self.byte_offset += 1;
                self.code_register = self
                    .code_register
                    .wrapping_add(0xfe00)
                    .wrapping_sub((self.current_byte() as u32) << 9);
                self.bits_available = 7;
            }
        } else {
            // Figure G.3: "BP = BP + 1; C = C + 0xFF00 - (B << 8); CT = 8"
            self.byte_offset += 1;
            self.code_register = self
                .code_register
                .wrapping_add(0xff00)
                .wrapping_sub((self.current_byte() as u32) << 8);
            self.bits_available = 8;
        }
    }

    /// The RENORMD procedure (E.3.3, Figure E.18).
    #[inline(always)]
    fn renormalize(&mut self) {
        loop {
            // Figure E.18: "CT = 0?"
            if self.bits_available == 0 {
                // Figure E.18: "BYTEIN"
                self.read_byte();
            }

            // Figure E.18: "A = A << 1; C = C << 1; CT = CT - 1"
            self.interval <<= 1;
            self.code_register <<= 1;
            self.bits_available -= 1;

            // Figure E.18: "A AND 0x8000 = 0?"
            if self.interval & 0x8000 != 0 {
                break;
            }
        }
    }

    /// The `LPS_EXCHANGE` procedure (E.3.2, Figure E.17).
    #[inline(always)]
    fn exchange_lps(&mut self, context: &mut Context, qe_entry: &QeData) -> u32 {
        // `D` - The decoded binary decision.
        let decoded_bit;

        // Figure E.17: "A < Qe(I(CX))?"
        if self.interval < qe_entry.probability {
            // Figure E.17: "A = Qe(I(CX)); D = MPS(CX); I(CX) = NMPS(I(CX))"
            self.interval = qe_entry.probability;
            decoded_bit = context.mps;
            context.state_index = qe_entry.next_index_on_mps;
        } else {
            // Figure E.17: "A = Qe(I(CX)); D = 1 - MPS(CX)"
            self.interval = qe_entry.probability;
            decoded_bit = 1 - context.mps;

            // Figure E.17: "SWITCH(I(CX)) = 1?"
            if qe_entry.switch_mps_sense {
                // Figure E.17: "MPS(CX) = 1 - MPS(CX)"
                context.mps = 1 - context.mps;
            }

            // Figure E.17: "I(CX) = NLPS(I(CX))"
            context.state_index = qe_entry.next_index_on_lps;
        }

        decoded_bit
    }

    /// The `MPS_EXCHANGE` procedure (E.3.2, Figure E.16).
    #[inline(always)]
    fn exchange_mps(&mut self, context: &mut Context, qe_entry: &QeData) -> u32 {
        // `D` - The decoded binary decision.
        let decoded_bit;

        // Figure E.16: "A < Qe(I(CX))?"
        if self.interval < qe_entry.probability {
            // Figure E.16: "D = 1 - MPS(CX)"
            decoded_bit = 1 - context.mps;

            // Figure E.16: "SWITCH(I(CX)) = 1?"
            if qe_entry.switch_mps_sense {
                // "MPS(CX) = 1 - MPS(CX)"
                context.mps = 1 - context.mps;
            }

            // Figure E.16: "I(CX) = NLPS(I(CX))"
            context.state_index = qe_entry.next_index_on_lps;
        } else {
            // Figure E.16: "D = MPS(CX); I(CX) = NMPS(I(CX))"
            decoded_bit = context.mps;
            context.state_index = qe_entry.next_index_on_mps;
        }

        decoded_bit
    }

    #[inline(always)]
    fn current_byte(&self) -> u8 {
        self.data
            .get(self.byte_offset as usize)
            .copied()
            .unwrap_or(0xFF)
    }

    #[inline(always)]
    fn next_byte(&self) -> u8 {
        self.data
            .get((self.byte_offset + 1) as usize)
            .copied()
            .unwrap_or(0xFF)
    }
}

/// Context for the arithmetic decoder.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Context {
    /// `I(CX)` - Index into the probability estimation state machine.
    pub(crate) state_index: u32,
    /// `MPS(CX)` - The sense of MPS for context CX.
    pub(crate) mps: u32,
}

/// Qe value table entry (Table E.1).
#[derive(Debug, Clone, Copy)]
struct QeData {
    /// `Qe` - The probability estimate value.
    probability: u32,
    /// `NMPS` - Next state index if MPS is coded.
    next_index_on_mps: u32,
    /// `NLPS` - Next state index if LPS is coded.
    next_index_on_lps: u32,
    /// `SWITCH` - Whether to flip the MPS sense.
    switch_mps_sense: bool,
}

macro_rules! qe {
    ($($qe:expr, $nmps:expr, $nlps:expr, $switch:expr),+ $(,)?) => {
        [
            $(
                QeData {
                    probability: $qe,
                    next_index_on_mps: $nmps,
                    next_index_on_lps: $nlps,
                    switch_mps_sense: $switch,
                }
            ),+
        ]
    };
}

/// "Table E.1 - Qe values and probability estimation process"
#[rustfmt::skip]
static QE_TABLE: [QeData; 47] = qe!(
    // Index  Qe_Value  NMPS  NLPS  SWITCH
    /*  0 */  0x5601,   1,    1,    true,
    /*  1 */  0x3401,   2,    6,    false,
    /*  2 */  0x1801,   3,    9,    false,
    /*  3 */  0x0AC1,   4,    12,   false,
    /*  4 */  0x0521,   5,    29,   false,
    /*  5 */  0x0221,   38,   33,   false,
    /*  6 */  0x5601,   7,    6,    true,
    /*  7 */  0x5401,   8,    14,   false,
    /*  8 */  0x4801,   9,    14,   false,
    /*  9 */  0x3801,   10,   14,   false,
    /* 10 */  0x3001,   11,   17,   false,
    /* 11 */  0x2401,   12,   18,   false,
    /* 12 */  0x1C01,   13,   20,   false,
    /* 13 */  0x1601,   29,   21,   false,
    /* 14 */  0x5601,   15,   14,   true,
    /* 15 */  0x5401,   16,   14,   false,
    /* 16 */  0x5101,   17,   15,   false,
    /* 17 */  0x4801,   18,   16,   false,
    /* 18 */  0x3801,   19,   17,   false,
    /* 19 */  0x3401,   20,   18,   false,
    /* 20 */  0x3001,   21,   19,   false,
    /* 21 */  0x2801,   22,   19,   false,
    /* 22 */  0x2401,   23,   20,   false,
    /* 23 */  0x2201,   24,   21,   false,
    /* 24 */  0x1C01,   25,   22,   false,
    /* 25 */  0x1801,   26,   23,   false,
    /* 26 */  0x1601,   27,   24,   false,
    /* 27 */  0x1401,   28,   25,   false,
    /* 28 */  0x1201,   29,   26,   false,
    /* 29 */  0x1101,   30,   27,   false,
    /* 30 */  0x0AC1,   31,   28,   false,
    /* 31 */  0x09C1,   32,   29,   false,
    /* 32 */  0x08A1,   33,   30,   false,
    /* 33 */  0x0521,   34,   31,   false,
    /* 34 */  0x0441,   35,   32,   false,
    /* 35 */  0x02A1,   36,   33,   false,
    /* 36 */  0x0221,   37,   34,   false,
    /* 37 */  0x0141,   38,   35,   false,
    /* 38 */  0x0111,   39,   36,   false,
    /* 39 */  0x0085,   40,   37,   false,
    /* 40 */  0x0049,   41,   38,   false,
    /* 41 */  0x0025,   42,   39,   false,
    /* 42 */  0x0015,   43,   40,   false,
    /* 43 */  0x0009,   44,   41,   false,
    /* 44 */  0x0005,   45,   42,   false,
    /* 45 */  0x0001,   45,   43,   false,
    /* 46 */  0x5601,   46,   46,   false,
);
