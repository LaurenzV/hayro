use crate::cancel::{Cancel, copy_work, row_mask};
use crate::decode::CombinationOperator;
use crate::error::{OverflowError, Result, bail};
use crate::memory::MemoryBudget;
use alloc::vec;
use alloc::vec::Vec;

// Keeps pixel coordinates plus offsets within `i32`.
pub(crate) const MAX_DIMENSION: u32 = 1 << 22;
const MAX_AREA: u64 = (u16::MAX as u64).pow(2);

pub(crate) type Word = u32;

pub(crate) const WORD_BITS: u32 = Word::BITS;
pub(crate) const WORD_BYTES: u32 = WORD_BITS / u8::BITS;
pub(crate) const WORD_SHIFT: u32 = WORD_BITS - 1;

#[derive(Debug, Clone)]
pub(crate) struct Bitmap {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: u32,
    pub(crate) data: Vec<Word>,
    pub(crate) x_location: u32,
    pub(crate) y_location: u32,
}

pub(crate) fn check_size(width: u32, height: u32) -> Result<()> {
    if width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_AREA
    {
        bail!(OverflowError::BitmapDimension);
    }

    Ok(())
}

impl Bitmap {
    pub(crate) fn new(width: u32, height: u32, budget: &mut MemoryBudget) -> Result<Self> {
        Self::new_with(width, height, 0, 0, false, budget)
    }

    pub(crate) fn new_with(
        width: u32,
        height: u32,
        x_location: u32,
        y_location: u32,
        default_pixel: bool,
        budget: &mut MemoryBudget,
    ) -> Result<Self> {
        check_size(width, height)?;

        let stride = width.div_ceil(WORD_BITS);
        let len = stride as usize * height as usize;
        budget.charge(Self::memory_size_for(len))?;

        let default_word: Word = if default_pixel { !0 } else { 0 };
        let data = vec![default_word; len];
        let mut bitmap = Self {
            width,
            height,
            stride,
            data,
            x_location,
            y_location,
        };

        if default_pixel {
            bitmap.clear_padding();
        }

        Ok(bitmap)
    }

    pub(crate) fn memory_size(&self) -> usize {
        Self::memory_size_for(self.data.len())
    }

    fn memory_size_for(words: usize) -> usize {
        words * WORD_BYTES as usize + size_of::<Self>()
    }

    fn clear_padding(&mut self) {
        let trailing = self.width % WORD_BITS;
        if trailing == 0 {
            return;
        }

        let mask: Word = !0 << (WORD_BITS - trailing);
        for row in self.data.chunks_exact_mut(self.stride as usize) {
            if let Some(last) = row.last_mut() {
                *last &= mask;
            }
        }
    }

    #[inline]
    pub(crate) fn get_pixel(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }

        let word_idx = (y * self.stride + x / WORD_BITS) as usize;
        let bit_pos = WORD_SHIFT - (x % WORD_BITS);
        ((self.data[word_idx] >> bit_pos) & 1) as u8
    }

    #[inline]
    pub(crate) fn set_pixel(&mut self, x: u32, y: u32, value: u8) {
        if x >= self.width || y >= self.height {
            return;
        }

        let word_idx = (y * self.stride + x / WORD_BITS) as usize;
        let bit_pos = WORD_SHIFT - (x % WORD_BITS);
        self.data[word_idx] |= (value as Word) << bit_pos;
    }

    pub(crate) fn combine(
        &mut self,
        other: &Self,
        x: i32,
        y: i32,
        operator: CombinationOperator,
        cancel: Cancel<'_>,
    ) -> Result<()> {
        // TODO: Optimize this, it's a bottleneck for symbols and patterns.
        let dest_x_start = x.max(0);
        let dest_x_end = x.saturating_add(other.width as i32).min(self.width as i32);

        if dest_x_start >= dest_x_end {
            return Ok(());
        }

        let src_x_start = (dest_x_start - x) as u32;
        let dest_x_start = dest_x_start as u32;
        let dest_x_end = dest_x_end as u32;

        let src_height = i64::from(other.height);
        let src_y_start = (-i64::from(y)).clamp(0, src_height) as u32;
        let src_y_end = (i64::from(self.height) - i64::from(y)).clamp(0, src_height) as u32;
        let check_mask = row_mask(copy_work(other.width));

        for src_y in src_y_start..src_y_end {
            if src_y & check_mask == check_mask {
                cancel.check()?;
            }

            let dest_y = (i64::from(y) + i64::from(src_y)) as u32;

            let first_word = dest_x_start / WORD_BITS;
            let last_word = (dest_x_end - 1) / WORD_BITS;

            for word_idx in first_word..=last_word {
                let word_start_x = word_idx * WORD_BITS;
                let word_end_x = word_start_x + WORD_BITS;

                let px_start = dest_x_start.max(word_start_x);
                let px_end = dest_x_end.min(word_end_x);

                let bit_start = px_start - word_start_x;
                let bit_end = px_end - word_start_x;

                let mask: Word = if bit_end == WORD_BITS {
                    !0 >> bit_start
                } else {
                    (!0 >> bit_start) & !(!0 >> bit_end)
                };

                let src_x_for_range = src_x_start + (px_start - dest_x_start);
                let src_word_idx = src_x_for_range / WORD_BITS;
                let src_bit_offset = src_x_for_range % WORD_BITS;

                let src_word1 = other.get_word(src_y, src_word_idx);
                let src_word2 = other.get_word(src_y, src_word_idx + 1);

                let src_raw = if src_bit_offset == 0 {
                    src_word1
                } else {
                    (src_word1 << src_bit_offset) | (src_word2 >> (WORD_BITS - src_bit_offset))
                };
                let src_aligned = src_raw >> bit_start;

                let dest_idx = (dest_y * self.stride + word_idx) as usize;
                let dest_word = self.data[dest_idx];

                let result = match operator {
                    CombinationOperator::Or => dest_word | src_aligned,
                    CombinationOperator::And => dest_word & src_aligned,
                    CombinationOperator::Xor => dest_word ^ src_aligned,
                    CombinationOperator::Xnor => !(dest_word ^ src_aligned),
                    CombinationOperator::Replace => src_aligned,
                };

                self.data[dest_idx] = (dest_word & !mask) | (result & mask);
            }
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn get_word(&self, row: u32, word_idx: u32) -> Word {
        if row >= self.height || word_idx >= self.stride {
            return 0;
        }

        self.data[(row * self.stride + word_idx) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_far_away() {
        let budget = &mut MemoryBudget::new(None);
        let mut page = Bitmap::new(8, 8, budget).unwrap();
        let region = Bitmap::new_with(8, 8, 0, 0, true, budget).unwrap();
        let cancel = Cancel(None);
        page.combine(&region, i32::MAX - 1, 0, CombinationOperator::Or, cancel)
            .unwrap();
        page.combine(&region, 0, i32::MAX - 1, CombinationOperator::Or, cancel)
            .unwrap();
        assert!(page.data.iter().all(|&w| w == 0));
    }
}
