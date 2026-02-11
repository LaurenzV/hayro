pub(super) struct Reader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> Reader<'a> {
    #[inline]
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    #[inline]
    pub(super) fn read_bit(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.byte_pos)?;
        let bit = (byte >> (7 - self.bit_pos)) & 1;

        self.bit_pos += 1;

        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }

        Some(bit)
    }

    #[inline]
    pub(super) fn read_u16(&mut self) -> Option<u16> {
        debug_assert_eq!(self.bit_pos, 0, "read_u16 called at non-byte boundary");

        let bytes: [u8; 2] = self
            .data
            .get(self.byte_pos..self.byte_pos + 2)?
            .try_into()
            .ok()?;
        self.byte_pos += 2;

        Some(u16::from_be_bytes(bytes))
    }

    #[inline]
    pub(super) fn read_u32(&mut self) -> Option<u32> {
        debug_assert_eq!(self.bit_pos, 0, "read_u32 called at non-byte boundary");

        let bytes: [u8; 4] = self
            .data
            .get(self.byte_pos..self.byte_pos + 4)?
            .try_into()
            .ok()?;
        self.byte_pos += 4;

        Some(u32::from_be_bytes(bytes))
    }

    #[inline]
    pub(super) fn read_u8(&mut self) -> Option<u8> {
        debug_assert_eq!(self.bit_pos, 0, "read_u8 called at non-byte boundary");

        let val = *self.data.get(self.byte_pos)?;
        self.byte_pos += 1;

        Some(val)
    }

    #[inline]
    pub(super) fn read_n_bytes(&mut self, n: usize) -> Option<u32> {
        debug_assert_eq!(
            self.bit_pos, 0,
            "read_n_bytes_be called at non-byte boundary"
        );

        match n {
            1 => Some(self.read_u8()? as u32),
            2 => Some(self.read_u16()? as u32),
            3 => {
                let bytes = self.read_bytes(3)?;
                Some((bytes[0] as u32) << 16 | (bytes[1] as u32) << 8 | bytes[2] as u32)
            }
            4 => self.read_u32(),
            _ => None,
        }
    }

    #[inline]
    pub(super) fn read_bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        debug_assert_eq!(self.bit_pos, 0, "read_bytes called at non-byte boundary");

        let bytes = self.data.get(self.byte_pos..self.byte_pos + n)?;
        self.byte_pos += n;

        Some(bytes)
    }

    #[inline]
    pub(super) fn position(&self) -> usize {
        self.byte_pos
    }

    #[inline]
    pub(super) fn at_end(&self) -> bool {
        self.byte_pos >= self.data.len()
    }

    #[inline]
    pub(super) fn eat_until(&mut self, f: impl Fn(u8) -> bool) -> &'a [u8] {
        debug_assert_eq!(self.bit_pos, 0, "eat_until called at non-byte boundary");

        let start = self.byte_pos;
        while let Some(&b) = self.data.get(self.byte_pos) {
            if f(b) {
                break;
            }
            self.byte_pos += 1;
        }

        &self.data[start..self.byte_pos]
    }
}
