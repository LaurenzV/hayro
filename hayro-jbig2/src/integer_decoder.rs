use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};

pub(crate) struct IntegerDecoder {
    contexts: Vec<ArithmeticDecoderContext>,
}

impl IntegerDecoder {
    #[inline(always)]
    pub(crate) fn new() -> Self {
        Self {
            contexts: vec![ArithmeticDecoderContext::default(); 512],
        }
    }

    #[inline(always)]
    pub(crate) fn decode(&mut self, decoder: &mut ArithmeticDecoder<'_>) -> Option<i32> {
        let mut prev: u32 = 1;

        let s = self.decode_bit(decoder, &mut prev);

        #[expect(
            clippy::same_functions_in_if_condition,
            reason = "each call mutates `prev`"
        )]
        let v = if self.decode_bit(decoder, &mut prev) == 0 {
            self.decode_n_bits(decoder, &mut prev, 2)
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            self.decode_n_bits(decoder, &mut prev, 4) + 4
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            self.decode_n_bits(decoder, &mut prev, 6) + 20
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            self.decode_n_bits(decoder, &mut prev, 8) + 84
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            self.decode_n_bits(decoder, &mut prev, 12) + 340
        } else {
            self.decode_n_bits(decoder, &mut prev, 32)
                .wrapping_add(4436)
        };

        if s == 0 {
            Some(v as i32)
        } else if v > 0 {
            Some((v as i32).wrapping_neg())
        } else {
            None
        }
    }

    #[inline(always)]
    fn decode_bit(&mut self, decoder: &mut ArithmeticDecoder<'_>, prev: &mut u32) -> u32 {
        let ctx_idx = (*prev & 0x1FF) as usize;
        let d = decoder.read_bit(&mut self.contexts[ctx_idx]);

        if *prev < 256 {
            *prev = (*prev << 1) | d;
        } else {
            *prev = (((*prev << 1) | d) & 511) | 256;
        }

        d
    }

    #[inline(always)]
    fn decode_n_bits(
        &mut self,
        decoder: &mut ArithmeticDecoder<'_>,
        prev: &mut u32,
        n: usize,
    ) -> u32 {
        let mut value = 0_u32;
        for _ in 0..n {
            let bit = self.decode_bit(decoder, prev);
            value = (value << 1) | bit;
        }
        value
    }
}
