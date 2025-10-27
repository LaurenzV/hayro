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
    context: DecoderContext,
}

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            c: 0,
            a: 0,
            bp: 0,
            ct: 0,
            context: DecoderContext { index: 0, mps: 0 },
        };
        
        // The INITDEC procedure from C.3.5.
        
        decoder.c = (decoder.b() as u32) << 16;
        decoder.byte_in();
        
        decoder.c = decoder.c << 7;
        decoder.ct = decoder.ct - 7;
        decoder.a = 0x8000;
        
        decoder
    }
    
    pub(crate) fn read_bit(&mut self, context: &DecoderContext) -> u32 {
        self.decode(context)
    }
    
    /// The BYTEIN procedure from C.3.4.
    fn byte_in(&mut self) {
        if self.b() == 0xff {
            let b1 = self.b1();
            
            if b1 > 0x8f {
                self.c = self.c + 0xff00;
                self.ct = 8;
            }   else {
                self.bp += 1;
                let b = self.b() as u32;
                self.c += b << 9;
                self.ct = 7;
            }
        }   else {
            self.bp += 1;
            let b = self.b() as u32;
            self.c = self.c + (b << 8);
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
    fn lps_exchange(&mut self) -> u32 {
        let d;
        
        let qe_entry = &QE_TABLE[self.context.index as usize];
        
        if self.a < qe_entry.qe {
            self.a = qe_entry.qe;
            d = self.context.mps;
            self.context.index = qe_entry.nmps;
        }   else {
            self.a = qe_entry.qe;
            d = 1 - self.context.mps;
            
            if qe_entry.switch {
                self.context.mps = 1 - self.context.mps;
            }
            
            self.context.index = qe_entry.nlps;
        }
        
        d
    }
    
    /// The DECODE procedure from C.3.2.
    fn decode(&mut self, context: &DecoderContext) -> u32 {
        self.context = *context;
        let qe_entry = &QE_TABLE[self.context.index as usize];
        
        self.a = self.a - qe_entry.qe;
        
        let d;
        
        if (self.c >> 16) < qe_entry.qe {
            d = self.lps_exchange();
            self.renorm_d();
        }   else {
            let mut c_high = self.c >> 16;
            let c_low = self.c & 0xffff;
            c_high = c_high - qe_entry.qe;
            
            self.c = (c_high << 16) | c_low;
            
            if self.a & 0x8000 == 0 {
                d = self.mps_exchange();
                self.renorm_d();
            }   else {
                d = self.context.mps;
            }
        }
        
        d
    }

    /// The MPS_EXCHANGE procedure from C.3.2.
    fn mps_exchange(&mut self) -> u32 {
        let d;

        let qe_entry = &QE_TABLE[self.context.index as usize];
        
        if self.a < qe_entry.qe {
            d = 1 - self.context.mps;
            
            if qe_entry.switch {
                self.context.mps = 1 - self.context.mps;
            }
            
            self.context.index = qe_entry.nlps;
        }   else {
            d = self.context.mps;
            self.context.index = qe_entry.nmps;
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

#[derive(Copy, Clone, Debug)]
pub(crate) struct DecoderContext {
    pub(crate) index: u32,
    pub(crate) mps: u32
}

#[derive(Debug, Clone, Copy)]
struct QeData {
    qe: u32,
    nmps: u32,
    nlps: u32,
    switch: bool,
}

/// QE values and associated data from Table C.2.
static QE_TABLE: [QeData; 47] = [
    QeData { qe: 0x5601, nmps: 1,  nlps: 1,  switch: true  },
    QeData { qe: 0x3401, nmps: 2,  nlps: 6,  switch: false },
    QeData { qe: 0x1801, nmps: 3,  nlps: 9,  switch: false },
    QeData { qe: 0x0AC1, nmps: 4,  nlps: 12, switch: false },
    QeData { qe: 0x0521, nmps: 5,  nlps: 29, switch: false },
    QeData { qe: 0x0221, nmps: 38, nlps: 33, switch: false },
    QeData { qe: 0x5601, nmps: 7,  nlps: 6,  switch: true  },
    QeData { qe: 0x5401, nmps: 8,  nlps: 14, switch: false },
    QeData { qe: 0x4801, nmps: 9,  nlps: 14, switch: false },
    QeData { qe: 0x3801, nmps: 10, nlps: 14, switch: false },
    QeData { qe: 0x3001, nmps: 11, nlps: 17, switch: false },
    QeData { qe: 0x2401, nmps: 12, nlps: 18, switch: false },
    QeData { qe: 0x1C01, nmps: 13, nlps: 20, switch: false },
    QeData { qe: 0x1601, nmps: 29, nlps: 21, switch: false },
    QeData { qe: 0x5601, nmps: 15, nlps: 14, switch: true  },
    QeData { qe: 0x5401, nmps: 16, nlps: 14, switch: false },
    QeData { qe: 0x5101, nmps: 17, nlps: 15, switch: false },
    QeData { qe: 0x4801, nmps: 18, nlps: 16, switch: false },
    QeData { qe: 0x3801, nmps: 19, nlps: 17, switch: false },
    QeData { qe: 0x3401, nmps: 20, nlps: 18, switch: false },
    QeData { qe: 0x3001, nmps: 21, nlps: 19, switch: false },
    QeData { qe: 0x2801, nmps: 22, nlps: 19, switch: false },
    QeData { qe: 0x2401, nmps: 23, nlps: 20, switch: false },
    QeData { qe: 0x2201, nmps: 24, nlps: 21, switch: false },
    QeData { qe: 0x1C01, nmps: 25, nlps: 22, switch: false },
    QeData { qe: 0x1801, nmps: 26, nlps: 23, switch: false },
    QeData { qe: 0x1601, nmps: 27, nlps: 24, switch: false },
    QeData { qe: 0x1401, nmps: 28, nlps: 25, switch: false },
    QeData { qe: 0x1201, nmps: 29, nlps: 26, switch: false },
    QeData { qe: 0x1101, nmps: 30, nlps: 27, switch: false },
    QeData { qe: 0x0AC1, nmps: 31, nlps: 28, switch: false },
    QeData { qe: 0x09C1, nmps: 32, nlps: 29, switch: false },
    QeData { qe: 0x08A1, nmps: 33, nlps: 30, switch: false },
    QeData { qe: 0x0521, nmps: 34, nlps: 31, switch: false },
    QeData { qe: 0x0441, nmps: 35, nlps: 32, switch: false },
    QeData { qe: 0x02A1, nmps: 36, nlps: 33, switch: false },
    QeData { qe: 0x0221, nmps: 37, nlps: 34, switch: false },
    QeData { qe: 0x0141, nmps: 38, nlps: 35, switch: false },
    QeData { qe: 0x0111, nmps: 39, nlps: 36, switch: false },
    QeData { qe: 0x0085, nmps: 40, nlps: 37, switch: false },
    QeData { qe: 0x0049, nmps: 41, nlps: 38, switch: false },
    QeData { qe: 0x0025, nmps: 42, nlps: 39, switch: false },
    QeData { qe: 0x0015, nmps: 43, nlps: 40, switch: false },
    QeData { qe: 0x0009, nmps: 44, nlps: 41, switch: false },
    QeData { qe: 0x0005, nmps: 45, nlps: 42, switch: false },
    QeData { qe: 0x0001, nmps: 46, nlps: 43, switch: false },
    QeData { qe: 0x5601, nmps: 46, nlps: 46, switch: false },
];

#[cfg(test)]
mod tests {
    use hayro_common::bit::BitWriter;
    use crate::arithmetic_decoder::{ArithmeticDecoder, DecoderContext};

    // Adapted from the Serenity decoder, which in turn took the example from 
    // https://www.itu.int/rec/T-REC-T.88-201808-I
    // H.2 Test sequence for arithmetic coder
    #[test]
    fn decode() {
        let input = [
            0x84, 0xC7, 0x3B, 0xFC, 0xE1, 0xA1, 0x43, 0x04,
            0x02, 0x20, 0x00, 0x00, 0x41, 0x0D, 0xBB, 0x86,
            0xF4, 0x31, 0x7F, 0xFF, 0x88, 0xFF, 0x37, 0x47,
            0x1A, 0xDB, 0x6A, 0xDF, 0xFF, 0xAC
        ];
        
        let expected_output = [
            0x00, 0x02, 0x00, 0x51, 0x00, 0x00, 0x00, 0xC0,
            0x03, 0x52, 0x87, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA,
            0x82, 0xC0, 0x20, 0x00, 0xFC, 0xD7, 0x9E, 0xF6,
            0xBF, 0x7F, 0xED, 0x90, 0x4F, 0x46, 0xA3, 0xBF
        ];
        
        let mut decoder = ArithmeticDecoder::new(&input[..]);
        
        let mut out_buf = vec![0; expected_output.len()];
        
        let mut writer = BitWriter::new(&mut out_buf, 1).unwrap();
        
        for _ in 0..expected_output.len() {
            for _ in 0..8 {
                writer.write(decoder.decode(&DecoderContext {index: 0, mps: 0}) as u16);
            }
        }
        
        assert_eq!(out_buf, expected_output);
    }
}
