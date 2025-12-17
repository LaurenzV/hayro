//! Bitmap representation for JBIG2 decoding.
//!
//! "The variable whose value is the result of this decoding procedure is shown
//! in Table 3." (6.2.3)
//!
//! "GBREG - The decoded region bitmap." (Table 3)

use crate::segment::region::CombinationOperator;

/// A decoded bitmap region.
///
/// Pixels are stored as booleans where `true` means black, `false` means white.
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
    /// Pixel data, one bool per pixel, row-major order.
    pub data: Vec<bool>,
}

impl Bitmap {
    /// Create a new bitmap filled with `false` (white pixels).
    pub fn new(width: u32, height: u32) -> Self {
        let data = vec![false; (width * height) as usize];
        Self {
            width,
            height,
            data,
        }
    }

    /// Get a pixel value at (x, y).
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.data[(y * self.width + x) as usize]
    }

    /// Set a pixel value at (x, y).
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, value: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.data[(y * self.width + x) as usize] = value;
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
                    CombinationOperator::Xnor => !(dst_pixel ^ src_pixel),
                    // "4 REPLACE"
                    CombinationOperator::Replace => src_pixel,
                };

                self.set_pixel(dest_x, dest_y, result);
            }
        }
    }
}
