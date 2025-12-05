//! The arithmetic decoder, described in Annex C.
//!
//! The arithmetic decoder keeps track of some state and continuously receives
//! context labels as input, each time yielding a new bit from the original data
//! as output.

pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying encoded data.
    data: &'a [u8],
    /// The C-register (see Table C.1).
    c: u32,
    /// The A-register (see Table C.1).
    a: u32,
    /// The pointer to the current byte.
    base_pointer: u32,
    /// The bit shift counter.
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

    /// Read the next bit using the given context label.
    #[inline(always)]
    pub(crate) fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        self.decode(context)
    }

    /// The INITDEC procedure from C.3.5.
    ///
    /// We use the version from Annex G in <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    fn initialize(&mut self) {
        self.c = ((self.current_byte() as u32) ^ 0xff) << 16;
        self.read_byte();

        self.c <<= 7;
        self.shift_count -= 7;
        self.a = 0x8000;
    }

    /// The BYTEIN procedure from C.3.4.
    ///
    /// We use the version from Annex G from <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    #[inline(always)]
    fn read_byte(&mut self) {
        if self.current_byte() == 0xff {
            let b1 = self.next_byte();

            if b1 > 0x8f {
                self.shift_count = 8;
            } else {
                self.base_pointer += 1;
                self.c = self.c + 0xfe00 - ((self.current_byte() as u32) << 9);
                self.shift_count = 7;
            }
        } else {
            self.base_pointer += 1;
            self.c = self.c + 0xff00 - ((self.current_byte() as u32) << 8);
            self.shift_count = 8;
        }
    }

    /// The RENORMD procedure from C.3.3.
    #[inline(always)]
    fn renormalize(&mut self) {
        loop {
            if self.shift_count == 0 {
                self.read_byte();
            }

            self.a <<= 1;
            self.c <<= 1;
            self.shift_count -= 1;

            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// The `LPS_EXCHANGE` procedure from C.3.2.
    #[inline(always)]
    fn exchange_lps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        if self.a < qe_entry.qe {
            self.a = qe_entry.qe;
            d = context.mps;
            context.index = qe_entry.nmps;
        } else {
            self.a = qe_entry.qe;
            d = 1 - context.mps;

            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            context.index = qe_entry.nlps;
        }

        d
    }

    /// The DECODE procedure from C.3.2.
    ///
    /// We use the version from Annex G from <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    #[inline(always)]
    fn decode(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index as usize];

        self.a -= qe_entry.qe;

        let d;

        if (self.c >> 16) < self.a {
            if self.a & 0x8000 == 0 {
                d = self.exchange_mps(context, qe_entry);
                self.renormalize();
            } else {
                d = context.mps;
            }
        } else {
            self.c -= self.a << 16;

            d = self.exchange_lps(context, qe_entry);
            self.renormalize();
        }

        d
    }

    /// The `MPS_EXCHANGE` procedure from C.3.2.
    #[inline(always)]
    fn exchange_mps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let d;

        if self.a < qe_entry.qe {
            d = 1 - context.mps;

            if qe_entry.switch {
                context.mps = 1 - context.mps;
            }

            context.index = qe_entry.nlps;
        } else {
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
            // "The number of bytes corresponding to the coding passes is
            // specified in the packet header. Often at that point there are
            // more symbols to be decoded. Therefore, the decoder shall extend
            // the input bit stream to the arithmetic coder with 0xFF bytes,
            // as necessary, until all symbols have been decoded."
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

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ArithmeticDecoderContext {
    pub(crate) index: u32,
    pub(crate) mps: u32,
}

#[derive(Debug, Clone, Copy)]
struct QeData {
    qe: u32,
    nmps: u32,
    nlps: u32,
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

/// QE values and associated data from Table C.2.
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
