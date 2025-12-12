use std::iter;
use log::{error, warn};
use crate::bit::BitReader;
use crate::tables::{Mode, EOFB};

mod bit;
mod decode;
mod tables;

#[derive(Copy, Clone, Debug)]
pub struct DecodeSettings {
    pub strict: bool,
    pub columns: u32,
    pub rows: u32,
    pub end_of_block: bool,
    pub end_of_line: bool,
}

pub trait Decoder {
    fn push_pixels(&mut self, count: usize, white: bool);
    fn next_line(&mut self);
}

pub fn decode(
    data: &[u8],
    decoder: &mut impl Decoder,
    settings: &DecodeSettings,
) -> Option<()> {
    let mut decoder = PrintDecoder::new();
    let mut ctx = DecoderContext::new(&mut decoder, settings);
    let mut reader = BitReader::new(data);
    
    loop {
        if settings.end_of_block {
            if reader.clone().read_bits(24) == Some(EOFB) {
                break;
            }
        }   else {
            if ctx.decoded_rows == settings.rows {
                break;
            }
        }
        
        let mode = reader.decode_mode()?;
        
        match mode {
            Mode::Pass => {
                ctx.push_pixels(ctx.b2 - ctx.a0());
                ctx.start_run();
            },
            Mode::Horizontal => {
                let a0a1 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a0a1);
                ctx.is_white = !ctx.is_white;
                let a1a2 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a1a2);
                ctx.is_white = !ctx.is_white;
                
                ctx.check_eol();
            },
            Mode::Vertical(i) => {
                let a1 = if i > 0 {
                    ctx.b1.checked_add(i as usize)?
                }   else {
                    ctx.b1.checked_sub((-i) as usize)?
                };
                
                ctx.push_pixels(a1 - ctx.a0());
                ctx.is_white = !ctx.is_white;
                
                ctx.check_eol();
            }
        }
    }
    
    Some(())
} 

struct DecoderContext<'a, T: Decoder> {
    /// The previous line.
    reference_line: Vec<u8>,
    /// The line we are currently decoding.
    coding_line: Vec<u8>,
    /// The decoder sink.
    decoder: &'a mut T,
    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    b1: usize,
    /// "The next changing element to the right of b1, on the reference line."
    b2: usize,
    /// The maximum permissible index for all variables.
    max_idx: usize,
    /// Whether the next run to be decoded is white.
    is_white: bool,
    
    decoded_rows: u32,
    settings: &'a DecodeSettings,
}

impl<'a, T: Decoder> DecoderContext<'a, T> {
    fn new(decoder: &'a mut T, settings: &'a DecodeSettings) -> DecoderContext<'a, T> {
        let max_idx = settings.columns as usize;
        let len = max_idx + 1;

        Self {
            // "The reference line for the first coding line in a
            // page is an imaginary white line."
            reference_line: vec![0; len],
            coding_line: vec![],
            decoder,
            b1: max_idx,
            b2: max_idx,
            max_idx,
            is_white: true,
            decoded_rows: 0,
            settings
        }
    }
    
    fn a0(&self) -> usize {
        self.coding_line.len()
    }
    
    fn find_b1(&mut self) {
        self.b1 = (self.a0() + 1).min(self.max_idx);
        let target_color = self.cur_color() ^ 1;
        
        let mut last_color =  self.reference_line[self.b1 - 1];
        
        while self.b1 < self.max_idx {
            let current_color = self.reference_line[self.b1];
            
            if current_color != last_color && current_color == target_color {
                break;
            }

            last_color = current_color;
            self.b1 += 1;
        }
    }

    fn find_b2(&mut self) {
        self.b2 = self.b1;
        
        let b1_color = self.reference_line[self.b1];

        while self.b2 < self.max_idx {
            if self.reference_line[self.b2] != b1_color {
                break;
            }
            
            self.b2 += 1;
        }
    }

    fn push_pixels(&mut self, count: usize) {
        self.decoder.push_pixels(count, self.is_white);
        self.coding_line.extend(iter::repeat_n(self.cur_color(), count))
    }
    
    fn cur_color(&self) -> u8 {
        if self.is_white {
            0
        }   else {
            1
        }
    }
    
    fn start_run(&mut self) {
        self.find_b1();
        self.find_b2();
    }

    fn check_eol(&mut self) {
        if self.a0() >= self.max_idx {
            // Go to next line.
            core::mem::swap(&mut self.reference_line, &mut self.coding_line);
            self.reference_line.resize(self.max_idx + 1, 0);
            self.coding_line.clear();
            self.is_white = true;
            self.decoded_rows += 1;
            self.decoder.next_line();
            
            self.start_run();
        }   else {
            self.start_run()
        }
    }
}

/// A decoder that prints the image to stdout.
pub struct PrintDecoder {
    line: String,
}

impl PrintDecoder {
    pub fn new() -> Self {
        Self { line: String::new() }
    }
}

impl Decoder for PrintDecoder {
    fn push_pixels(&mut self, count: usize, white: bool) {
        let symbol = if white { " " } else { "x" };
        for _ in 0..count {
            self.line.push_str(symbol);
        }
    }

    fn next_line(&mut self) {
        println!("{}", self.line);
        self.line.clear();
    }
}
