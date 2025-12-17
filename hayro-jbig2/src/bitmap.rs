//! Bitmap representation for JBIG2 decoding.
//!
//! "The variable whose value is the result of this decoding procedure is shown
//! in Table 3." (6.2.3)
//!
//! "GBREG - The decoded region bitmap." (Table 3)

use crate::segment::region::CombinationOperator;

/// A decoded bitmap region.
///
/// Pixels are stored as 1 bit per pixel, packed into bytes, with each row
/// byte-aligned. A pixel value of 1 means black, 0 means white.
///
/// "Pixels decoded by the MMR decoder having the value 'black' shall be treated
/// as having the value 1. Pixels decoded by the MMR decoder having the value
/// 'white' shall be treated as having the value 0." (6.2.6)
#[derive(Debug, Clone)]
pub(crate) struct Bitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel data, 1 bit per pixel, packed into bytes, rows byte-aligned.
    pub data: Vec<u8>,
}

impl Bitmap {
    /// Create a new bitmap filled with zeros (white pixels).
    pub fn new(width: u32, height: u32) -> Self {
        let stride = Self::compute_stride(width);
        let data = vec![0u8; stride * height as usize];
        Self {
            width,
            height,
            data,
        }
    }

    /// Compute the stride (bytes per row) for a given width.
    #[inline]
    pub fn compute_stride(width: u32) -> usize {
        (width as usize + 7) / 8
    }

    /// Returns the stride (number of bytes per row).
    #[inline]
    pub fn stride(&self) -> usize {
        Self::compute_stride(self.width)
    }

    /// Get a pixel value at (x, y). Returns 0 or 1.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let stride = self.stride();
        let byte_idx = y as usize * stride + (x as usize / 8);
        let bit_idx = 7 - (x as usize % 8);
        (self.data[byte_idx] >> bit_idx) & 1
    }

    /// Set a pixel value at (x, y). Value should be 0 or 1.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, value: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let stride = self.stride();
        let byte_idx = y as usize * stride + (x as usize / 8);
        let bit_idx = 7 - (x as usize % 8);
        if value != 0 {
            self.data[byte_idx] |= 1 << bit_idx;
        } else {
            self.data[byte_idx] &= !(1 << bit_idx);
        }
    }

    /// Combine another bitmap into this one at the given location using the
    /// specified combination operator.
    ///
    /// "These operators describe how the segment's bitmap is to be combined with
    /// the page bitmap." (7.4.1.5)
    pub fn combine(
        &mut self,
        other: &Bitmap,
        x_location: u32,
        y_location: u32,
        operator: CombinationOperator,
    ) {
        for y in 0..other.height {
            let dest_y = y_location + y;
            if dest_y >= self.height {
                break;
            }

            for x in 0..other.width {
                let dest_x = x_location + x;
                if dest_x >= self.width {
                    break;
                }

                let src_pixel = other.get_pixel(x, y);
                let dst_pixel = self.get_pixel(dest_x, dest_y);

                let result = match operator {
                    // "0 OR"
                    CombinationOperator::Or => dst_pixel | src_pixel,
                    // "1 AND"
                    CombinationOperator::And => dst_pixel & src_pixel,
                    // "2 XOR"
                    CombinationOperator::Xor => dst_pixel ^ src_pixel,
                    // "3 XNOR"
                    CombinationOperator::Xnor => !(dst_pixel ^ src_pixel) & 1,
                    // "4 REPLACE"
                    CombinationOperator::Replace => src_pixel,
                };

                self.set_pixel(dest_x, dest_y, result);
            }
        }
    }
}
