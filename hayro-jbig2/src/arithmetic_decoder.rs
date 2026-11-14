// Copy of hayro-jpeg2000's arithmetic decoder; keep the two in sync.

pub(crate) struct ArithmeticDecoder<'a> {
    data: &'a [u8],
    c: u32,
    a: u32,
    base_pointer: u32,
    shift_count: u32,
    markers_seen: u32,
}

// Encoders may trim trailing 0xFF 0x7F pairs, so valid data can hit the end marker many times.
const MAX_MARKERS: u32 = 65_536;

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            c: 0,
            a: 0,
            base_pointer: 0,
            shift_count: 0,
            markers_seen: 0,
        };

        decoder.initialize();

        decoder
    }

    #[inline(always)]
    pub(crate) fn is_exhausted(&self) -> bool {
        self.markers_seen > MAX_MARKERS
    }

    #[inline(always)]
    pub(crate) fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        self.decode(context)
    }

    fn initialize(&mut self) {
        self.c = ((self.current_byte() as u32) ^ 0xff) << 16;
        self.read_byte();

        self.c <<= 7;
        self.shift_count -= 7;
        self.a = 0x8000;
    }

    #[inline(always)]
    fn read_byte(&mut self) {
        if self.current_byte() == 0xff {
            let b1 = self.next_byte();

            if b1 > 0x8f {
                self.shift_count = 8;
                self.markers_seen = self.markers_seen.saturating_add(1);
            } else {
                self.base_pointer += 1;
                self.c = self
                    .c
                    .wrapping_add(0xfe00)
                    .wrapping_sub((self.current_byte() as u32) << 9);
                self.shift_count = 7;
            }
        } else {
            self.base_pointer += 1;
            self.c = self
                .c
                .wrapping_add(0xff00)
                .wrapping_sub((self.current_byte() as u32) << 8);
            self.shift_count = 8;
        }
    }

    #[inline(always)]
    fn renormalize(&mut self) {
        while self.a & 0x8000 == 0 {
            if self.shift_count == 0 {
                self.read_byte();
            }

            let shifts_needed = self.a.leading_zeros() - 16;
            let batch = shifts_needed.min(self.shift_count);
            self.a <<= batch;
            self.c <<= batch;
            self.shift_count -= batch;
        }
    }

    #[inline(always)]
    #[allow(unused, reason = "for reference")]
    fn exchange_lps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let cond = (self.a < qe_entry.qe) as u32;
        let inv_cond = 1 - cond;

        self.a = qe_entry.qe;
        let d = context.mps() ^ inv_cond;
        context.xor_mps(inv_cond & (qe_entry.switch as u32));
        let cond_u8 = cond as u8;
        let inv_cond_u8 = inv_cond as u8;
        context.set_index(cond_u8 * qe_entry.nmps + inv_cond_u8 * qe_entry.nlps);

        d
    }

    #[inline(always)]
    #[allow(unused, reason = "for reference")]
    fn exchange_mps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        let cond = (self.a < qe_entry.qe) as u32;
        let inv_cond = 1 - cond;
        let d = context.mps() ^ cond;
        context.xor_mps(cond & (qe_entry.switch as u32));
        let cond_u8 = cond as u8;
        let inv_cond_u8 = inv_cond as u8;
        context.set_index(cond_u8 * qe_entry.nlps + inv_cond_u8 * qe_entry.nmps);
        d
    }

    #[inline(always)]
    pub(crate) fn decode(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index() as usize];

        self.a -= qe_entry.qe;

        if (self.c >> 16) < self.a && self.a & 0x8000 != 0 {
            return context.mps();
        }

        let is_lps = ((self.c >> 16) >= self.a) as u32;

        let lps_mask = is_lps.wrapping_neg();
        self.c -= (self.a << 16) & lps_mask;

        let cond = (self.a < qe_entry.qe) as u32;

        self.a = (self.a & !lps_mask) | (qe_entry.qe & lps_mask);

        let d = context.mps() ^ cond ^ is_lps;

        context.xor_mps((cond ^ is_lps) & (qe_entry.switch as u32));

        let pick_nlps = ((cond ^ is_lps) as u8).wrapping_neg();
        context.set_index(qe_entry.nmps ^ ((qe_entry.nmps ^ qe_entry.nlps) & pick_nlps));

        self.renormalize();

        d
    }

    // Decodes `count` MPS at once if none would renormalize; otherwise decodes nothing and returns false.
    #[inline(always)]
    pub(crate) fn decode_mps_run(&mut self, context: ArithmeticDecoderContext, count: u32) -> bool {
        let total_qe = QE_TABLE[context.index() as usize].qe * count;
        let floor = ((self.c >> 16) + 1).max(0x8000);

        if self.a >= floor + total_qe {
            self.a -= total_qe;
            true
        } else {
            false
        }
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

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ArithmeticDecoderContext(u8);

impl ArithmeticDecoderContext {
    #[inline(always)]
    pub(crate) fn index(self) -> u32 {
        (self.0 & 0x7F) as u32
    }

    #[inline(always)]
    pub(crate) fn mps(self) -> u32 {
        (self.0 >> 7) as u32
    }

    #[inline(always)]
    fn set_index(&mut self, index: u8) {
        self.0 = (self.0 & 0x80) | index;
    }

    #[inline(always)]
    fn xor_mps(&mut self, val: u32) {
        self.0 ^= ((val & 1) << 7) as u8;
    }
}

#[derive(Debug, Clone, Copy)]
struct QeData {
    qe: u32,
    nmps: u8,
    nlps: u8,
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
