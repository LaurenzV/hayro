mod bit;

pub struct DecodeSettings {
    pub strict: bool,
    pub columns: u32,
    pub rows: u32,
    pub eoblock: bool,
}

pub trait Decoder {
    fn add_pixels(&mut self, number: u16, black: bool);
    fn end_of_line(&mut self);
}