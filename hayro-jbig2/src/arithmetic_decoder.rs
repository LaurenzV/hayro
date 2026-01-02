//! The arithmetic decoder (Annex E).
//!
//! "The arithmetic encoding procedure encodes a string of binary symbols.
//! The arithmetic decoding procedure receives an arithmetically coded bit
//! sequence and an associated sequence of context labels, and reconstructs
//! the original string of binary symbols." (E.1.1)
//!
//! The arithmetic decoder keeps track of some state and continuously receives
//! context labels as input, each time yielding a new bit from the original data
//! as output.

/// The arithmetic decoder state (E.3).
///
/// "State variables used by the arithmetic decoder procedures are described in
/// Table E.1." (E.3.1)
pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying encoded data.
    data: &'a [u8],
    /// "Chigh and Clow can be thought of as one 32-bit C-register" (E.3.1)
    c: u32,
    /// "A-register" (E.3.1)
    a: u32,
    /// "BP - A pointer to the compressed data"
    base_pointer: u32,
    /// "CT - The bit counter"
    shift_count: u32,
}

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            c: 0,
            a: 0,
            base_pointer: 0,
            shift_count: 0,
        };

        decoder.initialize();

        decoder
    }

    /// Read the next bit using the given context.
    #[inline(always)]
    pub(crate) fn decode(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        self.decode_internal(context)
    }

    /// The INITDEC procedure (E.3.5, Figure G.1).
    ///
    /// "The INITDEC procedure is used to start the arithmetic decoder."
    fn initialize(&mut self) {
        // Figure G.1: "C = (B XOR 0xFF) << 16"
        self.c = ((self.current_byte() as u32) ^ 0xff) << 16;

        // Figure G.1: "BYTEIN"
        self.read_byte();

        // Figure G.1: "C = C << 7; CT = CT - 7; A = 0x8000"
        self.c <<= 7;
        self.shift_count -= 7;
        self.a = 0x8000;
    }

    /// The BYTEIN procedure (E.3.4, Figure G.3).
    ///
    /// "The BYTEIN procedure called from RENORMD is illustrated in Figure E.19.
    /// This procedure reads in one byte of data, compensating for any stuff bits
    /// following the 0xFF byte in the process." (E.3.4)
    #[inline(always)]
    fn read_byte(&mut self) {
        // Figure G.3: "B = 0xFF?"
        if self.current_byte() == 0xff {
            let b1 = self.next_byte();

            // Figure G.3: "B1 > 0x8F?"
            // "If B1 exceeds 0x8F, then B1 must be one of the marker codes."
            if b1 > 0x8f {
                // Figure G.3: "CT = 8" (marker found, don't advance)
                self.shift_count = 8;
            } else {
                // Figure G.3: "BP = BP + 1; C = C + 0xFE00 - (B << 9); CT = 7"
                self.base_pointer += 1;
                self.c = self
                    .c
                    .wrapping_add(0xfe00)
                    .wrapping_sub((self.current_byte() as u32) << 9);
                self.shift_count = 7;
            }
        } else {
            // Figure G.3: "BP = BP + 1; C = C + 0xFF00 - (B << 8); CT = 8"
            self.base_pointer += 1;
            self.c = self
                .c
                .wrapping_add(0xff00)
                .wrapping_sub((self.current_byte() as u32) << 8);
            self.shift_count = 8;
        }
    }

    /// The RENORMD procedure (E.3.3, Figure E.18).
    ///
    /// "The RENORMD procedure for the decoder renormalization is illustrated in
    /// Figure E.18. A counter keeps track of the number of compressed bits in
    /// the Clow section of the C-register. When CT is zero, a new byte is
    /// inserted into Clow in the BYTEIN procedure." (E.3.3)
    #[inline(always)]
    fn renormalize(&mut self) {
        // Figure E.18: "Repeat ... Until A AND 0x8000 = 0?"
        loop {
            // Figure E.18: "CT = 0?" -> "BYTEIN"
            if self.shift_count == 0 {
                self.read_byte();
            }

            // Figure E.18: "A = A << 1; C = C << 1; CT = CT - 1"
            self.a <<= 1;
            self.c <<= 1;
            self.shift_count -= 1;

            // Figure E.18: "A AND 0x8000 = 0?" (exit when bit 15 is set)
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// The `LPS_EXCHANGE` procedure (E.3.2, Figure E.17).
    ///
    /// "For the LPS path of the decoder the conditional exchange procedure is
    /// given the `LPS_EXCHANGE` procedure shown in Figure E.17." (E.3.2)
    #[inline(always)]
    fn exchange_lps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        // Figure E.17: "A < Qe(I(CX))?"
        if self.a < qe_entry.qe {
            // Figure E.17 (Yes branch): "A = Qe(I(CX)); D = MPS(CX); I(CX) = NMPS(I(CX))"
            self.a = qe_entry.qe;
            d = context.mps;
            context.index = qe_entry.nmps;
        } else {
            // Figure E.17 (No branch): "A = Qe(I(CX)); D = 1 - MPS(CX)"
            self.a = qe_entry.qe;
            d = 1 - context.mps;

            // Figure E.17: "SWITCH(I(CX)) = 1?" -> "MPS(CX) = 1 - MPS(CX)"
            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            // Figure E.17: "I(CX) = NLPS(I(CX))"
            context.index = qe_entry.nlps;
        }

        d
    }

    /// The DECODE procedure (E.3.2, Figure G.2).
    ///
    /// "The decoder decodes one binary decision at a time. After decoding the
    /// decision, the decoder subtracts any amount from the code string that the
    /// encoder added." (E.3.2)
    #[inline(always)]
    fn decode_internal(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index as usize];

        // Figure G.2: "A = A - Qe(I(CX))"
        self.a -= qe_entry.qe;

        let d;

        // Figure G.2: "Chigh < A?"
        if (self.c >> 16) < self.a {
            // Figure G.2: "A AND 0x8000 = 0?"
            if self.a & 0x8000 == 0 {
                // Figure G.2: "D = MPS_EXCHANGE; RENORMD"
                d = self.exchange_mps(context, qe_entry);
                self.renormalize();
            } else {
                // Figure G.2: "D = MPS(CX)"
                d = context.mps;
            }
        } else {
            // Figure G.2: "Chigh = Chigh - A; D = LPS_EXCHANGE; RENORMD"
            self.c -= self.a << 16;

            d = self.exchange_lps(context, qe_entry);
            self.renormalize();
        }

        d
    }

    /// The `MPS_EXCHANGE` procedure (E.3.2, Figure E.16).
    ///
    /// "For the MPS path the conditional exchange procedure is shown in
    /// Figure E.16." (E.3.2)
    #[inline(always)]
    fn exchange_mps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        // Figure E.16: "A < Qe(I(CX))?"
        if self.a < qe_entry.qe {
            // Figure E.16 (Yes branch): "D = 1 - MPS(CX)"
            d = 1 - context.mps;

            // Figure E.16: "SWITCH(I(CX)) = 1?" -> "MPS(CX) = 1 - MPS(CX)"
            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            // Figure E.16: "I(CX) = NLPS(I(CX))"
            context.index = qe_entry.nlps;
        } else {
            // Figure E.16 (No branch): "D = MPS(CX); I(CX) = NMPS(I(CX))"
            d = context.mps;
            context.index = qe_entry.nmps;
        }

        d
    }

    #[inline(always)]
    fn current_byte(&self) -> u8 {
        self.data
            .get(self.base_pointer as usize)
            .copied()
            .unwrap_or(0xFF)
    }

    #[inline(always)]
    fn next_byte(&self) -> u8 {
        self.data
            .get((self.base_pointer + 1) as usize)
            .copied()
            .unwrap_or(0xFF)
    }
}

/// Arithmetic decoder context (E.2.4).
///
/// "Each context has associated with it an index, I(CX), which identifies a
/// particular probability estimate and its associated MPS value." (E.2.4)
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ArithmeticDecoderContext {
    /// "I(CX) - Index for context CX"
    pub(crate) index: u32,
    /// "MPS(CX) - The sense of MPS for context CX"
    pub(crate) mps: u32,
}

/// Qe value table entry (Table E.1).
#[derive(Debug, Clone, Copy)]
struct QeData {
    /// "`Qe_Value`" - The probability estimate
    qe: u32,
    /// "NMPS" - Next index if MPS is coded
    nmps: u32,
    /// "NLPS" - Next index if LPS is coded
    nlps: u32,
    /// "SWITCH" - MPS/LPS symbol switch
    switch: bool,
}

macro_rules! qe {
    ($($qe:expr, $nmps:expr, $nlps:expr, $switch:expr),+ $(,)?) => {
        [
            $(
                QeData {
                    qe: $qe,
                    nmps: $nmps,
                    nlps: $nlps,
                    switch: $switch,
                }
            ),+
        ]
    };
}

/// "Table E.1 - Qe values and probability estimation process"
#[rustfmt::skip]
static QE_TABLE: [QeData; 47] = qe!(
    // Index  Qe_Value  NMPS  NLPS  SWITCH
    /*  0 */ 0x5601,    1,    1,    true,
    /*  1 */ 0x3401,    2,    6,    false,
    /*  2 */ 0x1801,    3,    9,    false,
    /*  3 */ 0x0AC1,    4,    12,   false,
    /*  4 */ 0x0521,    5,    29,   false,
    /*  5 */ 0x0221,    38,   33,   false,
    /*  6 */ 0x5601,    7,    6,    true,
    /*  7 */ 0x5401,    8,    14,   false,
    /*  8 */ 0x4801,    9,    14,   false,
    /*  9 */ 0x3801,    10,   14,   false,
    /* 10 */ 0x3001,    11,   17,   false,
    /* 11 */ 0x2401,    12,   18,   false,
    /* 12 */ 0x1C01,    13,   20,   false,
    /* 13 */ 0x1601,    29,   21,   false,
    /* 14 */ 0x5601,    15,   14,   true,
    /* 15 */ 0x5401,    16,   14,   false,
    /* 16 */ 0x5101,    17,   15,   false,
    /* 17 */ 0x4801,    18,   16,   false,
    /* 18 */ 0x3801,    19,   17,   false,
    /* 19 */ 0x3401,    20,   18,   false,
    /* 20 */ 0x3001,    21,   19,   false,
    /* 21 */ 0x2801,    22,   19,   false,
    /* 22 */ 0x2401,    23,   20,   false,
    /* 23 */ 0x2201,    24,   21,   false,
    /* 24 */ 0x1C01,    25,   22,   false,
    /* 25 */ 0x1801,    26,   23,   false,
    /* 26 */ 0x1601,    27,   24,   false,
    /* 27 */ 0x1401,    28,   25,   false,
    /* 28 */ 0x1201,    29,   26,   false,
    /* 29 */ 0x1101,    30,   27,   false,
    /* 30 */ 0x0AC1,    31,   28,   false,
    /* 31 */ 0x09C1,    32,   29,   false,
    /* 32 */ 0x08A1,    33,   30,   false,
    /* 33 */ 0x0521,    34,   31,   false,
    /* 34 */ 0x0441,    35,   32,   false,
    /* 35 */ 0x02A1,    36,   33,   false,
    /* 36 */ 0x0221,    37,   34,   false,
    /* 37 */ 0x0141,    38,   35,   false,
    /* 38 */ 0x0111,    39,   36,   false,
    /* 39 */ 0x0085,    40,   37,   false,
    /* 40 */ 0x0049,    41,   38,   false,
    /* 41 */ 0x0025,    42,   39,   false,
    /* 42 */ 0x0015,    43,   40,   false,
    /* 43 */ 0x0009,    44,   41,   false,
    /* 44 */ 0x0005,    45,   42,   false,
    /* 45 */ 0x0001,    45,   43,   false,
    /* 46 */ 0x5601,    46,   46,   false,
);
