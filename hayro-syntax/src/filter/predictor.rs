use crate::filter::png::{self, RowFilter};
use crate::object::Dict;
use crate::object::dict::keys::{BITS_PER_COMPONENT, COLORS, COLUMNS, PREDICTOR};
use alloc::vec::Vec;

pub(crate) fn apply(data: Vec<u8>, params: &Dict<'_>) -> Option<Vec<u8>> {
    let predictor = params.get::<u32>(PREDICTOR).unwrap_or(1);

    if predictor == 1 {
        return Some(data);
    }

    let colors = params.get::<usize>(COLORS).unwrap_or(1);
    let bits_per_component = params.get::<usize>(BITS_PER_COMPONENT).unwrap_or(8);
    let columns = params.get::<usize>(COLUMNS).unwrap_or(1);

    if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
        return None;
    }

    let bits_per_pixel = colors.checked_mul(bits_per_component)?;
    let row_len = columns.checked_mul(bits_per_pixel)?.div_ceil(8);

    if row_len == 0 {
        return None;
    }

    match predictor {
        2 => Some(tiff(data, colors, bits_per_component, columns, row_len)),
        10..=15 => Some(png(data, bits_per_pixel.div_ceil(8), row_len)),
        _ => None,
    }
}

fn tiff(
    mut data: Vec<u8>,
    colors: usize,
    bits_per_component: usize,
    columns: usize,
    row_len: usize,
) -> Vec<u8> {
    for row in data.chunks_mut(row_len) {
        match bits_per_component {
            8 => png::unfilter_row(RowFilter::Sub, colors, &[], row),
            16 => {
                let distance = 2 * colors;

                for i in (distance..row.len() - 1).step_by(2) {
                    let left = u16::from_be_bytes([row[i - distance], row[i - distance + 1]]);
                    let value = u16::from_be_bytes([row[i], row[i + 1]]).wrapping_add(left);
                    row[i..i + 2].copy_from_slice(&value.to_be_bytes());
                }
            }
            _ => {
                let per_byte = 8 / bits_per_component;
                let mask = (1_u8 << bits_per_component) - 1;
                let shift = |k: usize| 8 - bits_per_component * (k % per_byte + 1);
                let num_components = (columns * colors).min(row.len() * per_byte);

                for k in colors..num_components {
                    let left = row[(k - colors) / per_byte] >> shift(k - colors);
                    let byte = &mut row[k / per_byte];
                    let value = (*byte >> shift(k)).wrapping_add(left) & mask;
                    *byte = (*byte & !(mask << shift(k))) | (value << shift(k));
                }
            }
        }
    }

    data
}

fn png(mut data: Vec<u8>, bytes_per_pixel: usize, row_len: usize) -> Vec<u8> {
    let mut src = 0;
    let mut dst = 0;

    while src < data.len() {
        let filter = data[src];
        let len = row_len.min(data.len() - src - 1);
        data.copy_within(src + 1..src + 1 + len, dst);

        let (done, rest) = data.split_at_mut(dst);
        let previous = &done[done.len().saturating_sub(row_len)..];

        if let Some(filter) = RowFilter::from_u8(filter) {
            png::unfilter_row(filter, bytes_per_pixel, previous, &mut rest[..len]);
        }

        src += row_len + 1;
        dst += len;
    }

    data.truncate(dst);

    data
}
