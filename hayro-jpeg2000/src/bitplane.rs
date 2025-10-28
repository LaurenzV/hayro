use crate::packet::{CodeBlock, SubbandType};

pub(crate) struct DecodeContext {
    signs: Vec<u8>,
    magnitude_array: Vec<u8>,
    significance_states: Vec<u8>,
    first_magnitude_refinement: Vec<u8>,
    eta: Vec<u8>,
    width: u32,
    height: u32,
}

impl DecodeContext {
    pub(crate) fn new() -> Self {
        Self {
            signs: vec![],
            magnitude_array: vec![],
            significance_states: vec![],
            first_magnitude_refinement: vec![],
            eta: vec![],
            width: 0,
            height: 0,
        }
    }

    pub(crate) fn reset(&mut self, width: u32, height: u32) {
        for arr in [
            &mut self.signs,
            &mut self.magnitude_array,
            &mut self.significance_states,
            &mut self.first_magnitude_refinement,
            &mut self.eta,
        ] {
            arr.clear();
            arr.resize(width as usize * height as usize, 0);
        }

        self.width = width;
        self.height = height;
    }

    fn significance_state(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            0
        } else {
            self.significance_states[x as usize + y as usize * self.width as usize]
        }
    }
    
    fn is_significant(&self, x: i64, y: i64) -> bool {
        self.significance_state(x, y) != 0
    }
    
    fn is_magnitude_refined(&self, x: i64, y: i64) -> bool {
        self.magnitude_array[x as usize + y as usize * self.width as usize] != 0
    }

    fn sign(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            0
        } else {
            self.signs[x as usize + y as usize * self.width as usize]
        }
    }

    /// The horizontal reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn horizontal_reference(&self, x: u32, y: u32) -> u8 {
        self.significance_state(x as i64 - 1, y as i64)
            + self.significance_state(x as i64 + 1, y as i64)
    }

    /// The vertical reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn vertical_reference(&self, x: u32, y: u32) -> u8 {
        self.significance_state(x as i64, y as i64 - 1)
            + self.significance_state(x as i64, y as i64 + 1)
    }

    /// The diagonal reference value for computing the context for significance
    /// propagation and cleanup pass.
    fn diagonal_reference(&self, x: u32, y: u32) -> u8 {
        self.significance_state(x as i64 - 1, y as i64 - 1)
            + self.significance_state(x as i64 + 1, y as i64 - 1)
            + self.significance_state(x as i64 - 1, y as i64 + 1)
            + self.significance_state(x as i64 + 1, y as i64 + 1)
    }
}

pub(crate) fn decode(code_block: &mut CodeBlock) -> Option<()> {
    Some(())
}

fn context_label_zero_coding(x: u32, y: u32, ctx: &DecodeContext, subband_type: SubbandType) -> u8 {
    let horizontal = ctx.horizontal_reference(x, y);
    let vertical = ctx.vertical_reference(x, y);
    let diagonal = ctx.diagonal_reference(x, y);

    match subband_type {
        SubbandType::LowLow | SubbandType::LowHigh => {
            if horizontal == 2 {
                8
            } else if horizontal == 1 && vertical >= 1 {
                7
            } else if horizontal == 1 && vertical == 0 && diagonal >= 1 {
                6
            } else if horizontal == 1 && vertical == 0 && diagonal == 0 {
                5
            } else if horizontal == 0 && vertical == 2 {
                4
            } else if horizontal == 0 && vertical == 1 {
                3
            } else if horizontal == 0 && vertical == 0 && diagonal >= 2 {
                2
            } else if horizontal == 0 && vertical == 0 && diagonal == 1 {
                1
            } else {
                0
            }
        }
        SubbandType::HighLow => {
            if vertical == 2 {
                8
            } else if horizontal >= 1 && vertical == 1 {
                7
            } else if horizontal == 0 && vertical == 1 && diagonal >= 1 {
                6
            } else if horizontal == 0 && vertical == 1 && diagonal == 0 {
                5
            } else if horizontal == 2 && vertical == 0 {
                4
            } else if horizontal == 1 && vertical == 0 {
                3
            } else if horizontal == 0 && vertical == 0 && diagonal >= 2 {
                2
            } else if horizontal == 0 && vertical == 0 && diagonal == 1 {
                1
            } else {
                0
            }
        }
        SubbandType::HighHigh => {
            let hv = horizontal + vertical;

            if diagonal >= 3 {
                8
            } else if hv >= 1 && diagonal == 2 {
                7
            } else if hv == 0 && diagonal == 2 {
                6
            } else if hv >= 2 && diagonal == 1 {
                5
            } else if hv == 1 && diagonal == 1 {
                4
            } else if hv == 0 && diagonal == 1 {
                3
            } else if hv >= 2 && diagonal == 0 {
                2
            } else if hv == 1 && diagonal == 0 {
                1
            } else {
                0
            }
        }
    }
}

fn context_label_sign_coding(x: u32, y: u32, ctx: &DecodeContext) -> u8 {
    fn neighbor_contribution(ctx: &DecodeContext, x: i64, y: i64) -> i32 {
        let sigma = ctx.significance_state(x, y);

        let multiplied = if ctx.sign(x, y) == 0 { 1 } else { -1 };

        multiplied * sigma as i32
    }

    let h = (neighbor_contribution(ctx, x as i64 - 1, y as i64)
        + neighbor_contribution(ctx, x as i64 + 1, y as i64))
    .clamp(-1, 1);
    let v = (neighbor_contribution(ctx, x as i64, y as i64 - 1)
        + neighbor_contribution(ctx, x as i64, y as i64 + 1))
    .clamp(-1, 1);

    match (h, v) {
        (1, 1) => 13,
        (1, 0) => 12,
        (1, -1) => 11,
        (0, 1) => 10,
        (0, 0) => 9,
        (0, -1) => 10,
        (-1, 1) => 11,
        (-1, 0) => 12,
        (-1, -1) => 13,
        _ => unreachable!(),
    }
}

fn context_label_magnitude_refinement_coding(x: u32, y: u32, ctx: &DecodeContext) -> u8 {
    if ctx.is_magnitude_refined(x as i64, y as i64) {
        16
    }   else {
        let x = x as i64;
        let y = y as i64;
        let summed = ctx.significance_state(x - 1, y) 
            + ctx.significance_state(x + 1, y)
        + ctx.significance_state(x - 1, y - 1)
        + ctx.significance_state(x - 1, y + 1)
        + ctx.significance_state(x + 1, y - 1)
        + ctx.significance_state(x + 1, y + 1);
        
        if summed >= 1 {
            15
        }   else {
            14
        }
    }
}
