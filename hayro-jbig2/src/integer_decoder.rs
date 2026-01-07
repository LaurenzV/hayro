use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};

/// Integer arithmetic decoder (Annex A.2).
///
/// "An invocation of an arithmetic integer decoding procedure involves decoding
/// a sequence of bits, where each bit is decoded using a context formed by the
/// bits decoded previously in this invocation." (A.1)
///
/// "Each context for each arithmetic integer decoding procedure has its own
/// adaptive probability estimate used by the underlying arithmetic coder." (A.1)
pub(crate) struct IntegerDecoder {
    /// "Each arithmetic integer decoding procedure requires 512 bytes of storage
    /// for its context memory." (A.2)
    contexts: Vec<Context>,
}

impl IntegerDecoder {
    /// Create a new integer decoder with fresh contexts.
    pub(crate) fn new() -> Self {
        Self {
            contexts: vec![Context::default(); 512],
        }
    }

    /// Decode a signed integer (Annex A.2).
    ///
    /// Returns `Some(value)` on success, or `None` if OOB (out-of-band) is decoded.
    ///
    /// "The result of the integer arithmetic decoding procedure is equal to:
    /// - V if S = 0
    /// - -V if S = 1 and V > 0
    /// - OOB if S = 1 and V = 0" (A.2)
    pub(crate) fn decode(&mut self, decoder: &mut ArithmeticDecoder<'_>) -> Option<i32> {
        // "1) Set: PREV = 1" (A.2)
        let mut prev: u32 = 1;

        // "2) Follow the flowchart in Figure A.1. Decode each bit with CX equal
        // to 'IAx + PREV' where 'IAx' represents the identifier of the current
        // arithmetic integer decoding procedure, '+' represents concatenation,
        // and the rightmost 9 bits of PREV are used." (A.2)

        // Decode S (sign bit)
        let s = self.decode_bit(decoder, &mut prev);

        // Follow Figure A.1 flowchart to decode V
        #[expect(
            clippy::same_functions_in_if_condition,
            reason = "each call mutates `prev`"
        )]
        let v = if self.decode_bit(decoder, &mut prev) == 0 {
            // "V = next 2 bits" (values 0-3)
            self.decode_n_bits(decoder, &mut prev, 2)
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            // "V = (next 4 bits) + 4" (values 4-19)
            self.decode_n_bits(decoder, &mut prev, 4) + 4
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            // "V = (next 6 bits) + 20" (values 20-83)
            self.decode_n_bits(decoder, &mut prev, 6) + 20
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            // "V = (next 8 bits) + 84" (values 84-339)
            self.decode_n_bits(decoder, &mut prev, 8) + 84
        } else if self.decode_bit(decoder, &mut prev) == 0 {
            // "V = (next 12 bits) + 340" (values 340-4435)
            self.decode_n_bits(decoder, &mut prev, 12) + 340
        } else {
            // "V = (next 32 bits) + 4436" (values 4436+)
            self.decode_n_bits(decoder, &mut prev, 32) + 4436
        };

        // "The result of the integer arithmetic decoding procedure is equal to:
        // - V if S = 0
        // - -V if S = 1 and V > 0
        // - OOB if S = 1 and V = 0" (A.2)
        if s == 0 {
            Some(v as i32)
        } else if v > 0 {
            Some(-(v as i32))
        } else {
            // "OOB if S = 1 and V = 0"
            None
        }
    }

    /// Decode a single bit and update PREV.
    ///
    /// "3) After each bit is decoded: If PREV < 256 set:
    ///     PREV = (PREV << 1) OR D
    /// Otherwise set:
    ///     PREV = (((PREV << 1) OR D) AND 511) OR 256
    /// where D represents the value of the just-decoded bit." (A.2)
    #[inline]
    fn decode_bit(&mut self, decoder: &mut ArithmeticDecoder<'_>, prev: &mut u32) -> u32 {
        let ctx_idx = (*prev & 0x1FF) as usize;
        let d = decoder.decode(&mut self.contexts[ctx_idx]);

        // Update PREV according to step 3
        if *prev < 256 {
            *prev = (*prev << 1) | d;
        } else {
            // "PREV always contains the values of the eight most-recently-decoded
            // bits, plus a leading 1 bit, which is used to indicate the number of
            // bits decoded so far." (A.2)
            *prev = (((*prev << 1) | d) & 511) | 256;
        }

        d
    }

    /// Decode n bits and update PREV for each.
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
