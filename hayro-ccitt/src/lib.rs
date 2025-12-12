use std::iter;
use log::{error, warn};
use crate::bit::BitReader;
use crate::tables::Mode;

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
        let mode = reader.decode_mode()?;
        
        eprintln!("{:?}", mode);
        
        match mode {
            Mode::Pass => unimplemented!(),
            Mode::Horizontal => {
                let h = reader.read_bits(3)?;
                
                if h != 0b001 {
                    error!("invalid code word for horizontal mode");
                    
                    return None;
                }
                
                let a0a1 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a0a1);
                ctx.is_white = !ctx.is_white;
                let a1a2 = reader.decode_run(ctx.is_white)? as usize;
                ctx.push_pixels(a1a2);
                ctx.is_white = !ctx.is_white;
                
                ctx.a0 += a0a1 + a1a2;
                
                unimplemented!()
            },
            Mode::Vertical(i) => {
                let a1 = if i > 0 {
                    ctx.b1.checked_add(i as usize)?
                }   else {
                    ctx.b1.checked_sub((-i) as usize)?
                };
                
                if a1 > ctx.max_idx {
                    error!("a1 was too large");
                    
                    return None;
                }
                
                if a1 < ctx.a0 {
                    error!("a1 has an invalid position.");
                    
                    return None;
                }
                
                ctx.push_pixels(a1 - ctx.a0);
                ctx.a0 = a1;
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
    /// "The reference or starting changing element on the coding line."
    a0: usize,
    /// "The first changing element on the reference line to the right of a0 and
    /// of opposite color to a0."
    b1: usize,
    /// "The next changing element to the right of b1, on the reference line."
    b2: usize,
    /// The maximum permissible index for all variables.
    max_idx: usize,
    /// Whether the next run to be decoded is white.
    is_white: bool,
    settings: &'a DecodeSettings,
}

impl<'a, T: Decoder> DecoderContext<'a, T> {
    fn new(decoder: &'a mut T, settings: &'a DecodeSettings) -> DecoderContext<'a, T> {
        // +1 so that we can emulate the imaginary first white element.
        let max_idx = settings.columns as usize + 1;

        Self {
            // "The reference line for the first coding line in a
            // page is an imaginary white line."
            reference_line: vec![0; max_idx],
            coding_line: vec![0],
            decoder,
            a0: 0,
            a1: max_idx,
            a2: max_idx,
            b1: max_idx,
            b2: max_idx,
            max_idx,
            is_white: true,
            settings
        }
    }

    fn push_pixels(&mut self, count: usize) {
        self.decoder.push_pixels(count, self.is_white);
        let val = if self.is_white { 0 } else { 1 };
        self.coding_line.extend(iter::repeat_n(val, count))
    }
    
    fn next_iteration(&mut self) {}

    fn check_eol(&mut self) {
        if self.a0 == self.max_idx {
            // Go to next line.
            self.a0 = 0;
            core::mem::swap(&mut self.reference_line, &mut self.coding_line);
            self.coding_line.truncate(1);
            self.is_white = true;
            
            self.decoder.next_line();
        }
    }
    
    fn is_eol(&self) -> bool {
        self.a0 == self.max_idx
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
    fn push_pixels(&mut self, count: usize, black: bool) {
        let symbol = if black { "x" } else { "o" };
        for _ in 0..count {
            self.line.push_str(symbol);
        }
    }

    fn next_line(&mut self) {
        println!("{}", self.line);
        self.line.clear();
    }
}
