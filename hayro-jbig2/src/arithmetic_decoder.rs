//! The arithmetic decoder.
//!
//! "The arithmetic encoding procedure encodes a string of binary symbols.
//! The arithmetic decoding procedure receives an arithmetically coded bit
//! sequence and an associated sequence of context labels, and reconstructs
//! the original string of binary symbols." (E.1.1)
//!
//! The arithmetic decoder keeps track of some state and continuously receives
//! context labels as input, each time yielding a new bit from the original data
//! as output.
//!
//! Note: The references in this file (e.g., "Annex E", "Table E.1", "Figure E.15")
//! refer to the JPEG 2000 spec (ITU-T T.800). Equivalent references can be found
//! in the JBIG2 spec (ITU-T T.88) in Annex E as well, as both standards use the
//! same MQ arithmetic coder.

/// The arithmetic decoder state (E.3).
///
/// "State variables used by the arithmetic decoder procedures are described in
/// Table E.1." (E.3.1)
pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying encoded data.
    data: &'a [u8],
    /// "C - The Clow/Chigh register" (Table E.1)
    c: u32,
    /// "A - The A-register (current value of the probability interval)" (Table E.1)
    a: u32,
    /// "BP - A pointer to the compressed data" (Table E.1)
    base_pointer: u32,
    /// "CT - The bit counter" (Table E.1)
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

    /// The INITDEC procedure (E.3.5).
    ///
    /// "The INITDEC procedure is used to start the arithmetic decoder."
    fn initialize(&mut self) {
        // "INITDEC initialises the registers, in the order shown in Figure E.20."
        //
        // "B = B + 1" - Move to first byte (we start at 0)
        // "C = (B << 16)" - Load first byte into high bits of C
        self.c = ((self.current_byte() as u32) ^ 0xff) << 16;
        self.read_byte();

        // "C = (C << 7)"
        self.c <<= 7;
        // "CT = CT - 7"
        self.shift_count -= 7;
        // "A = 0x8000"
        self.a = 0x8000;
    }

    /// The BYTEIN procedure (E.3.4).
    ///
    /// "The BYTEIN procedure fetches a new byte of compressed data and adds it
    /// to the C-register."
    #[inline(always)]
    fn read_byte(&mut self) {
        // "if B = 0xFF then" (Figure E.19)
        if self.current_byte() == 0xff {
            let b1 = self.next_byte();

            // "if B1 > 0x8F then"
            // "This is either a marker code, or a bit-stuff of 0 that should be
            // skipped over."
            if b1 > 0x8f {
                // "CT = 8" - marker found, don't advance, just reset counter
                self.shift_count = 8;
            } else {
                // "BP = BP + 1"
                self.base_pointer += 1;
                // "C = C + 0xFE00 - (B << 9)"
                self.c = self
                    .c
                    .wrapping_add(0xfe00)
                    .wrapping_sub((self.current_byte() as u32) << 9);
                // "CT = 7"
                self.shift_count = 7;
            }
        } else {
            // "BP = BP + 1"
            self.base_pointer += 1;
            // "C = C + 0xFF00 - (B << 8)"
            self.c = self
                .c
                .wrapping_add(0xff00)
                .wrapping_sub((self.current_byte() as u32) << 8);
            // "CT = 8"
            self.shift_count = 8;
        }
    }

    /// The RENORMD procedure (E.3.3).
    ///
    /// "The RENORMD procedure is used to renormalise A and C, reading a new byte
    /// if necessary."
    #[inline(always)]
    fn renormalize(&mut self) {
        // "Repeat ... Until A >= 0x8000" (Figure E.18)
        loop {
            // "if CT = 0 then BYTEIN"
            if self.shift_count == 0 {
                self.read_byte();
            }

            // "A = A << 1"
            self.a <<= 1;
            // "C = C << 1"
            self.c <<= 1;
            // "CT = CT - 1"
            self.shift_count -= 1;

            // "Until A >= 0x8000"
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// The LPS_EXCHANGE procedure (E.3.2).
    ///
    /// "LPS_EXCHANGE is invoked when Chigh < A after the probability interval
    /// has been reduced by Qe(CX)."
    #[inline(always)]
    fn exchange_lps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        // "if A < Qe(CX) then" (Figure E.17)
        if self.a < qe_entry.qe {
            // "A = Qe(CX)"
            self.a = qe_entry.qe;
            // "D = MPS(CX)"
            d = context.mps;
            // "I(CX) = NMPS(I(CX))"
            context.index = qe_entry.nmps;
        } else {
            // "A = Qe(CX)"
            self.a = qe_entry.qe;
            // "D = 1 - MPS(CX)"
            d = 1 - context.mps;

            // "if SWITCH(I(CX)) = 1 then MPS(CX) = 1 - MPS(CX)"
            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            // "I(CX) = NLPS(I(CX))"
            context.index = qe_entry.nlps;
        }

        d
    }

    /// The DECODE procedure (E.3.2).
    ///
    /// "The DECODE procedure decodes a binary decision by reading the compressed
    /// data bit by bit."
    #[inline(always)]
    fn decode_internal(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index as usize];

        // "A = A - Qe(CX)" (Figure E.15)
        self.a -= qe_entry.qe;

        let d;

        // "if Chigh < A then"
        if (self.c >> 16) < self.a {
            // "if A < 0x8000 then MPS_EXCHANGE else D = MPS(CX)"
            if self.a & 0x8000 == 0 {
                d = self.exchange_mps(context, qe_entry);
                self.renormalize();
            } else {
                d = context.mps;
            }
        } else {
            // "Chigh = Chigh - A"
            self.c -= self.a << 16;

            // "LPS_EXCHANGE"
            d = self.exchange_lps(context, qe_entry);
            self.renormalize();
        }

        d
    }

    /// The MPS_EXCHANGE procedure (E.3.2).
    ///
    /// "MPS_EXCHANGE is invoked when A < 0x8000 after the probability interval
    /// has been reduced by Qe(CX)."
    #[inline(always)]
    fn exchange_mps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        // "if A < Qe(CX) then" (Figure E.16)
        if self.a < qe_entry.qe {
            // "D = 1 - MPS(CX)"
            d = 1 - context.mps;

            // "if SWITCH(I(CX)) = 1 then MPS(CX) = 1 - MPS(CX)"
            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            // "I(CX) = NLPS(I(CX))"
            context.index = qe_entry.nlps;
        } else {
            // "D = MPS(CX)"
            d = context.mps;
            // "I(CX) = NMPS(I(CX))"
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
/// particular probability estimate and its associated MPS value. This index
/// is used to access a probability estimation state machine." (E.2.4)
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ArithmeticDecoderContext {
    /// "I(CX) - Index for context CX" (Table E.1)
    pub(crate) index: u32,
    /// "MPS(CX) - The sense of MPS for context CX" (Table E.1)
    pub(crate) mps: u32,
}

/// Qe value table entry.
#[derive(Debug, Clone, Copy)]
struct QeData {
    /// "Qe(I(CX)) - Current Qe value at row I(CX) in Table E.13" (Table E.1)
    qe: u32,
    /// "NMPS(I) - Next index if MPS is coded" (Table E.13)
    nmps: u32,
    /// "NLPS(I) - Next index if LPS is coded" (Table E.13)
    nlps: u32,
    /// "SWITCH(I) - MPS/LPS symbol switch" (Table E.13)
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

/// "Qe values and probability estimation state machine" (Table E.13)
#[rustfmt::skip]
static QE_TABLE: [QeData; 47] = qe!(
    0x5601, 1, 1, true,
    0x3401, 2, 6, false,
    0x1801, 3, 9, false,
    0x0AC1, 4, 12, false,
    0x0521, 5, 29, false,
    0x0221, 38, 33, false,
    0x5601, 7, 6, true,
    0x5401, 8, 14, false,
    0x4801, 9, 14, false,
    0x3801, 10, 14, false,
    0x3001, 11, 17, false,
    0x2401, 12, 18, false,
    0x1C01, 13, 20, false,
    0x1601, 29, 21, false,
    0x5601, 15, 14, true,
    0x5401, 16, 14, false,
    0x5101, 17, 15, false,
    0x4801, 18, 16, false,
    0x3801, 19, 17, false,
    0x3401, 20, 18, false,
    0x3001, 21, 19, false,
    0x2801, 22, 19, false,
    0x2401, 23, 20, false,
    0x2201, 24, 21, false,
    0x1C01, 25, 22, false,
    0x1801, 26, 23, false,
    0x1601, 27, 24, false,
    0x1401, 28, 25, false,
    0x1201, 29, 26, false,
    0x1101, 30, 27, false,
    0x0AC1, 31, 28, false,
    0x09C1, 32, 29, false,
    0x08A1, 33, 30, false,
    0x0521, 34, 31, false,
    0x0441, 35, 32, false,
    0x02A1, 36, 33, false,
    0x0221, 37, 34, false,
    0x0141, 38, 35, false,
    0x0111, 39, 36, false,
    0x0085, 40, 37, false,
    0x0049, 41, 38, false,
    0x0025, 42, 39, false,
    0x0015, 43, 40, false,
    0x0009, 44, 41, false,
    0x0005, 45, 42, false,
    0x0001, 45, 43, false,
    0x5601, 46, 46, false,
);
