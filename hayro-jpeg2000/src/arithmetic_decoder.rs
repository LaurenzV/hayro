//! The arithmetic decoder, described in Annex C.

pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying data.
    data: &'a [u8],
    /// The C-register, as illustrated in table C.1.
    c: u32,
    /// The A-register, as illustrated in table C.1.
    a: u32,
    /// The pointer to the current byte.
    bp: u32,
    /// The bit counter.
    ct: u32,
}

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            c: 0,
            a: 0,
            bp: 0,
            ct: 0,
        };

        // The INITDEC procedure from C.3.5.
        // We use the version from Annex G from https://www.itu.int/rec/T-REC-T.88-201808-I.

        decoder.c = ((decoder.b() as u32) ^ 0xff) << 16;
        decoder.byte_in();

        decoder.c = decoder.c << 7;
        decoder.ct = decoder.ct - 7;
        decoder.a = 0x8000;

        decoder
    }

    pub(crate) fn read_bit(&mut self, context: &mut DecoderContext) -> u32 {
        self.decode(context)
    }

    /// The BYTEIN procedure from C.3.4.
    /// We use the version from Annex G from https://www.itu.int/rec/T-REC-T.88-201808-I.
    fn byte_in(&mut self) {
        if self.b() == 0xff {
            let b1 = self.b1();

            if b1 > 0x8f {
                self.ct = 8;
            } else {
                self.bp += 1;
                self.c = self.c + 0xfe00 - ((self.b() as u32) << 9);
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c = self.c + 0xff00 - ((self.b() as u32) << 8);
            self.ct = 8;
        }
    }

    /// The RENORMD procedure from C.3.3.
    fn renorm_d(&mut self) {
        loop {
            if self.ct == 0 {
                self.byte_in();
            }

            self.a = self.a << 1;
            self.c = self.c << 1;
            self.ct -= 1;

            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// The LPS_EXCHANGE procedure from C.3.2.
    fn lps_exchange(&mut self, context: &mut DecoderContext) -> u32 {
        let d;

        let qe_entry = &QE_TABLE[context.index as usize];

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
    /// We use the version from Annex G from https://www.itu.int/rec/T-REC-T.88-201808-I.
    fn decode(&mut self, context: &mut DecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index as usize];

        self.a = self.a - qe_entry.qe;

        let d;

        if (self.c >> 16) < self.a {
            if self.a & 0x8000 == 0 {
                d = self.mps_exchange(context);
                self.renorm_d();
            } else {
                d = context.mps;
            }
        } else {
            let mut c_high = self.c >> 16;
            let c_low = self.c & 0xffff;
            c_high = c_high - self.a;

            self.c = (c_high << 16) | c_low;

            d = self.lps_exchange(context);
            self.renorm_d();
        }

        d
    }

    /// The MPS_EXCHANGE procedure from C.3.2.
    fn mps_exchange(&mut self, context: &mut DecoderContext) -> u32 {
        let d;

        let qe_entry = &QE_TABLE[context.index as usize];

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

    fn b(&self) -> u8 {
        self.data[self.bp as usize]
    }

    fn b1(&self) -> u8 {
        self.data[(self.bp + 1) as usize]
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct DecoderContext {
    pub(crate) index: u32,
    pub(crate) mps: u32,
}

impl DecoderContext {
    pub(crate) fn new(index: u32, mps: u32) -> Self {
        Self { index, mps }
    }
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
    0x0001, 46, 43, false,
    0x5601, 46, 46, false,
);

#[cfg(test)]
mod tests {
    use crate::arithmetic_decoder::{ArithmeticDecoder, DecoderContext};
    use hayro_common::bit::BitWriter;

    // Adapted from the Serenity decoder, which in turn took the example from
    // https://www.itu.int/rec/T-REC-T.88-201808-I
    // H.2 Test sequence for arithmetic coder.
    #[test]
    fn decode() {
        let input = [
            0x84, 0xC7, 0x3B, 0xFC, 0xE1, 0xA1, 0x43, 0x04, 0x02, 0x20, 0x00, 0x00, 0x41, 0x0D,
            0xBB, 0x86, 0xF4, 0x31, 0x7F, 0xFF, 0x88, 0xFF, 0x37, 0x47, 0x1A, 0xDB, 0x6A, 0xDF,
            0xFF, 0xAC,
        ];

        let expected_output = [
            0x00, 0x02, 0x00, 0x51, 0x00, 0x00, 0x00, 0xC0, 0x03, 0x52, 0x87, 0x2A, 0xAA, 0xAA,
            0xAA, 0xAA, 0x82, 0xC0, 0x20, 0x00, 0xFC, 0xD7, 0x9E, 0xF6, 0xBF, 0x7F, 0xED, 0x90,
            0x4F, 0x46, 0xA3, 0xBF,
        ];

        let mut decoder = ArithmeticDecoder::new(&input[..]);
        let mut out_buf = vec![0; expected_output.len()];
        let mut ctx = DecoderContext::default();

        let mut writer = BitWriter::new(&mut out_buf, 1).unwrap();

        for _ in 0..expected_output.len() {
            for _ in 0..8 {
                let next = decoder.decode(&mut ctx) as u16;
                writer.write(next);
            }
        }

        assert_eq!(out_buf, expected_output);
    }
}
