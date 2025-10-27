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
        
        decoder.c = (decoder.b() as u32) << 16;
        decoder.byte_in();
        
        decoder.c = decoder.c << 7;
        decoder.ct = decoder.ct - 7;
        decoder.a = 0x8000;
        
        decoder
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
    
    fn b(&self) -> u8 {
        self.data[self.bp as usize]
    }
    
    fn b1(&self) -> u8 {
        self.data[(self.bp + 1) as usize]
    }
}

#[derive(Debug, Clone, Copy)]
struct QeData {
    value: u16,
    nmps: u8,
    nlps: u8,
    switch_flag: bool,
}

/// QE values and associated data from Table C.2.
static QE_TABLE: [QeData; 47] = [
    QeData { value: 0x5601, nmps: 1,  nlps: 1,  switch_flag: true  },
    QeData { value: 0x3401, nmps: 2,  nlps: 6,  switch_flag: false },
    QeData { value: 0x1801, nmps: 3,  nlps: 9,  switch_flag: false },
    QeData { value: 0x0AC1, nmps: 4,  nlps: 12, switch_flag: false },
    QeData { value: 0x0521, nmps: 5,  nlps: 29, switch_flag: false },
    QeData { value: 0x0221, nmps: 38, nlps: 33, switch_flag: false },
    QeData { value: 0x5601, nmps: 7,  nlps: 6,  switch_flag: true  },
    QeData { value: 0x5401, nmps: 8,  nlps: 14, switch_flag: false },
    QeData { value: 0x4801, nmps: 9,  nlps: 14, switch_flag: false },
    QeData { value: 0x3801, nmps: 10, nlps: 14, switch_flag: false },
    QeData { value: 0x3001, nmps: 11, nlps: 17, switch_flag: false },
    QeData { value: 0x2401, nmps: 12, nlps: 18, switch_flag: false },
    QeData { value: 0x1C01, nmps: 13, nlps: 20, switch_flag: false },
    QeData { value: 0x1601, nmps: 29, nlps: 21, switch_flag: false },
    QeData { value: 0x5601, nmps: 15, nlps: 14, switch_flag: true  },
    QeData { value: 0x5401, nmps: 16, nlps: 14, switch_flag: false },
    QeData { value: 0x5101, nmps: 17, nlps: 15, switch_flag: false },
    QeData { value: 0x4801, nmps: 18, nlps: 16, switch_flag: false },
    QeData { value: 0x3801, nmps: 19, nlps: 17, switch_flag: false },
    QeData { value: 0x3401, nmps: 20, nlps: 18, switch_flag: false },
    QeData { value: 0x3001, nmps: 21, nlps: 19, switch_flag: false },
    QeData { value: 0x2801, nmps: 22, nlps: 19, switch_flag: false },
    QeData { value: 0x2401, nmps: 23, nlps: 20, switch_flag: false },
    QeData { value: 0x2201, nmps: 24, nlps: 21, switch_flag: false },
    QeData { value: 0x1C01, nmps: 25, nlps: 22, switch_flag: false },
    QeData { value: 0x1801, nmps: 26, nlps: 23, switch_flag: false },
    QeData { value: 0x1601, nmps: 27, nlps: 24, switch_flag: false },
    QeData { value: 0x1401, nmps: 28, nlps: 25, switch_flag: false },
    QeData { value: 0x1201, nmps: 29, nlps: 26, switch_flag: false },
    QeData { value: 0x1101, nmps: 30, nlps: 27, switch_flag: false },
    QeData { value: 0x0AC1, nmps: 31, nlps: 28, switch_flag: false },
    QeData { value: 0x09C1, nmps: 32, nlps: 29, switch_flag: false },
    QeData { value: 0x08A1, nmps: 33, nlps: 30, switch_flag: false },
    QeData { value: 0x0521, nmps: 34, nlps: 31, switch_flag: false },
    QeData { value: 0x0441, nmps: 35, nlps: 32, switch_flag: false },
    QeData { value: 0x02A1, nmps: 36, nlps: 33, switch_flag: false },
    QeData { value: 0x0221, nmps: 37, nlps: 34, switch_flag: false },
    QeData { value: 0x0141, nmps: 38, nlps: 35, switch_flag: false },
    QeData { value: 0x0111, nmps: 39, nlps: 36, switch_flag: false },
    QeData { value: 0x0085, nmps: 40, nlps: 37, switch_flag: false },
    QeData { value: 0x0049, nmps: 41, nlps: 38, switch_flag: false },
    QeData { value: 0x0025, nmps: 42, nlps: 39, switch_flag: false },
    QeData { value: 0x0015, nmps: 43, nlps: 40, switch_flag: false },
    QeData { value: 0x0009, nmps: 44, nlps: 41, switch_flag: false },
    QeData { value: 0x0005, nmps: 45, nlps: 42, switch_flag: false },
    QeData { value: 0x0001, nmps: 46, nlps: 43, switch_flag: false },
    QeData { value: 0x5601, nmps: 46, nlps: 46, switch_flag: false },
];
