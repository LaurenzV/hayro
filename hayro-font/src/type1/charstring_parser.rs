use crate::Builder;
use crate::OutlineError;
use crate::argstack::ArgumentsStack;
use crate::type1::stream::Stream;

pub(crate) struct CharStringParser<'a> {
    pub stack: ArgumentsStack<'a>,
    pub builder: &'a mut Builder<'a>,
    pub sbx: f32,
    pub x: f32,
    pub y: f32,
    pub is_flexing: bool,
}

impl CharStringParser<'_> {
    #[inline]
    pub fn parse_move_to(&mut self) -> Result<(), OutlineError> {
        if self.is_flexing {
            return Ok(());
        }

        self.x += self.stack.at(0);
        self.y += self.stack.at(1);
        self.builder.move_to(self.x, self.y);

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_horizontal_move_to(&mut self) -> Result<(), OutlineError> {
        if self.is_flexing {
            self.stack.push(0.0)?;
            return Ok(());
        }

        self.x += self.stack.at(0);
        self.builder.move_to(self.x, self.y);

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_vertical_move_to(&mut self) -> Result<(), OutlineError> {
        if self.is_flexing {
            self.stack.push(0.0)?;
            self.stack.exch();
            return Ok(());
        }

        self.y += self.stack.at(0);
        self.builder.move_to(self.x, self.y);

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_line_to(&mut self) -> Result<(), OutlineError> {
        let mut i = 0;
        while i < self.stack.len() {
            self.x += self.stack.at(i);
            self.y += self.stack.at(i + 1);
            self.builder.line_to(self.x, self.y);
            i += 2;
        }

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_horizontal_line_to(&mut self) -> Result<(), OutlineError> {
        let mut i = 0;
        while i < self.stack.len() {
            self.x += self.stack.at(i);
            i += 1;
            self.builder.line_to(self.x, self.y);

            if i == self.stack.len() {
                break;
            }

            self.y += self.stack.at(i);
            i += 1;
            self.builder.line_to(self.x, self.y);
        }

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_vertical_line_to(&mut self) -> Result<(), OutlineError> {
        let mut i = 0;
        while i < self.stack.len() {
            self.y += self.stack.at(i);
            i += 1;
            self.builder.line_to(self.x, self.y);

            if i == self.stack.len() {
                break;
            }

            self.x += self.stack.at(i);
            i += 1;
            self.builder.line_to(self.x, self.y);
        }

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_curve_to(&mut self) -> Result<(), OutlineError> {
        let mut i = 0;
        while i < self.stack.len() {
            let x1 = self.x + self.stack.at(i);
            let y1 = self.y + self.stack.at(i + 1);
            let x2 = x1 + self.stack.at(i + 2);
            let y2 = y1 + self.stack.at(i + 3);
            self.x = x2 + self.stack.at(i + 4);
            self.y = y2 + self.stack.at(i + 5);

            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            i += 6;
        }

        self.stack.clear();
        Ok(())
    }

    #[inline]
    pub fn parse_hv_curve_to(&mut self) -> Result<(), OutlineError> {
        if self.stack.len() < 4 {
            return Err(OutlineError::InvalidArgumentsStackLength);
        }

        self.stack.reverse();
        while !self.stack.is_empty() {
            if self.stack.len() < 4 {
                return Err(OutlineError::InvalidArgumentsStackLength);
            }

            let x1 = self.x + self.stack.pop();
            let y1 = self.y;
            let x2 = x1 + self.stack.pop();
            let y2 = y1 + self.stack.pop();
            self.y = y2 + self.stack.pop();
            self.x = x2;
            if self.stack.len() == 1 {
                self.x += self.stack.pop();
            }
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            if self.stack.is_empty() {
                break;
            }

            if self.stack.len() < 4 {
                return Err(OutlineError::InvalidArgumentsStackLength);
            }

            let x1 = self.x;
            let y1 = self.y + self.stack.pop();
            let x2 = x1 + self.stack.pop();
            let y2 = y1 + self.stack.pop();
            self.x = x2 + self.stack.pop();
            self.y = y2;
            if self.stack.len() == 1 {
                self.y += self.stack.pop()
            }
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
        }

        debug_assert!(self.stack.is_empty());
        Ok(())
    }

    #[inline]
    pub fn parse_vh_curve_to(&mut self) -> Result<(), OutlineError> {
        if self.stack.len() < 4 {
            return Err(OutlineError::InvalidArgumentsStackLength);
        }

        self.stack.reverse();
        while !self.stack.is_empty() {
            if self.stack.len() < 4 {
                return Err(OutlineError::InvalidArgumentsStackLength);
            }

            let x1 = self.x;
            let y1 = self.y + self.stack.pop();
            let x2 = x1 + self.stack.pop();
            let y2 = y1 + self.stack.pop();
            self.x = x2 + self.stack.pop();
            self.y = y2;
            if self.stack.len() == 1 {
                self.y += self.stack.pop();
            }
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            if self.stack.is_empty() {
                break;
            }

            if self.stack.len() < 4 {
                return Err(OutlineError::InvalidArgumentsStackLength);
            }

            let x1 = self.x + self.stack.pop();
            let y1 = self.y;
            let x2 = x1 + self.stack.pop();
            let y2 = y1 + self.stack.pop();
            self.y = y2 + self.stack.pop();
            self.x = x2;
            if self.stack.len() == 1 {
                self.x += self.stack.pop();
            }
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
        }

        debug_assert!(self.stack.is_empty());
        Ok(())
    }

    // Copied from fonttools.
    #[inline]
    pub fn parse_flex(&mut self) -> Result<(), OutlineError> {
        let final_y = self.stack.pop();
        let final_x = self.stack.pop();
        let _ = self.stack.pop(); // Ignored

        let p3y = self.stack.pop();
        let p3x = self.stack.pop();
        let bcp4y = self.stack.pop();
        let bcp4x = self.stack.pop();
        let bcp3y = self.stack.pop();
        let bcp3x = self.stack.pop();
        let p2y = self.stack.pop();
        let p2x = self.stack.pop();
        let bcp2y = self.stack.pop();
        let bcp2x = self.stack.pop();
        let bcp1y = self.stack.pop();
        let bcp1x = self.stack.pop();
        let rpy = self.stack.pop();
        let rpx = self.stack.pop();

        self.stack.push(bcp1x + rpx)?;
        self.stack.push(bcp1y + rpy)?;
        self.stack.push(bcp2x)?;
        self.stack.push(bcp2y)?;
        self.stack.push(p2x)?;
        self.stack.push(p2y)?;
        self.parse_curve_to()?;

        self.stack.push(bcp3x)?;
        self.stack.push(bcp3y)?;
        self.stack.push(bcp4x)?;
        self.stack.push(bcp4y)?;
        self.stack.push(p3x)?;
        self.stack.push(p3y)?;
        self.parse_curve_to()?;

        // Push final position back on the stack
        self.stack.push(final_x)?;
        self.stack.push(final_y)?;

        Ok(())
    }

    #[inline]
    pub fn parse_close_path(&mut self) -> Result<(), OutlineError> {
        self.builder.close();

        Ok(())
    }

    #[inline]
    pub fn parse_int1(&mut self, op: u8) -> Result<(), OutlineError> {
        let n = i16::from(op) - 139;
        self.stack.push(f32::from(n))?;
        Ok(())
    }

    #[inline]
    pub fn parse_int2(&mut self, op: u8, s: &mut Stream) -> Result<(), OutlineError> {
        let b1 = s.read_byte().ok_or(OutlineError::ReadOutOfBounds)?;
        let n = (i16::from(op) - 247) * 256 + i16::from(b1) + 108;
        debug_assert!((108..=1131).contains(&n));
        self.stack.push(f32::from(n))?;
        Ok(())
    }

    #[inline]
    pub fn parse_int3(&mut self, op: u8, s: &mut Stream) -> Result<(), OutlineError> {
        let b1 = s.read_byte().ok_or(OutlineError::ReadOutOfBounds)?;
        let n = -(i16::from(op) - 251) * 256 - i16::from(b1) - 108;
        debug_assert!((-1131..=-108).contains(&n));
        self.stack.push(f32::from(n))?;
        Ok(())
    }

    #[inline]
    pub fn parse_int4(&mut self, s: &mut Stream) -> Result<(), OutlineError> {
        let b = s.read_bytes(4).ok_or(OutlineError::ReadOutOfBounds)?;
        let num = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);

        // Make sure number is in-range.
        debug_assert!((num as f32 as i32) == num);

        self.stack.push(num as f32)?;
        Ok(())
    }
}
