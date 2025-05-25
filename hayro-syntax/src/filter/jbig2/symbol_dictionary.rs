use crate::filter::jbig2::bitmap::decode_bitmap;
use crate::filter::jbig2::refinement::decode_refinement;
use crate::filter::jbig2::standard_table::get_standard_table;
use crate::filter::jbig2::text_region::decode_text_region;
use crate::filter::jbig2::{
    Bitmap, DecodingContext, Jbig2Error, Reader, SymbolDictionaryHuffmanTables, TemplatePixel,
    decode_mmr_bitmap, log2, read_uncompressed_bitmap,
};

// 6.5.5 Decoding the symbol dictionary
pub(crate) fn decode_symbol_dictionary(
    huffman: bool,
    refinement: bool,
    symbols: &[Bitmap],
    number_of_new_symbols: usize,
    _number_of_exported_symbols: usize,
    huffman_tables: Option<&SymbolDictionaryHuffmanTables>,
    template_index: usize,
    at: &[TemplatePixel],
    refinement_template_index: usize,
    refinement_at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
    mut huffman_input: Option<&Reader>,
) -> Result<Vec<Bitmap>, Jbig2Error> {
    if huffman && refinement {
        return Err(Jbig2Error::new(
            "symbol refinement with Huffman is not supported",
        ));
    }

    let mut new_symbols = Vec::new();
    let mut current_height = 0i32;
    let mut symbol_code_length = log2(symbols.len() + number_of_new_symbols);

    let table_b1 = if huffman {
        Some(get_standard_table(1)?) // standard table B.1
    } else {
        None
    };
    let mut symbol_widths = Vec::new();
    if huffman {
        symbol_code_length = symbol_code_length.max(1); // 6.5.8.2.3
    }

    while new_symbols.len() < number_of_new_symbols {
        // Delta height decoding
        let delta_height = if huffman {
            huffman_tables
                .as_ref()
                .ok_or_else(|| Jbig2Error::new("Huffman tables required"))?
                .height_table
                .decode(
                    huffman_input
                        .as_ref()
                        .ok_or_else(|| Jbig2Error::new("Huffman input required"))?,
                )
                .map_err(|_| Jbig2Error::new("Failed to decode delta height"))?
                .ok_or_else(|| Jbig2Error::new("Got OOB for delta height"))?
        } else {
            decoding_context
                .decode_integer("IADH") // 6.5.6
                .ok_or_else(|| Jbig2Error::new("Failed to decode IADH"))?
        };
        current_height += delta_height;

        let mut current_width = 0i32;
        let mut total_width = 0i32;
        let first_symbol = if huffman { symbol_widths.len() } else { 0 };

        loop {
            let delta_width = if huffman {
                let result = huffman_tables
                    .as_ref()
                    .unwrap()
                    .width_table
                    .decode(huffman_input.clone().unwrap())?;

                result
            } else {
                decoding_context.decode_integer("IADW") // 6.5.7
            };

            let Some(dw) = delta_width else { break }; // OOB
            current_width += dw;
            total_width += current_width;

            if refinement {
                // 6.5.8.2 Refinement/aggregate-coded symbol bitmap
                let number_of_instances = decoding_context
                    .decode_integer("IAAI")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode IAAI"))?;

                let bitmap = if number_of_instances > 1 {
                    // Multiple instances - call text region
                    let mut all_symbols = symbols.to_vec();
                    all_symbols.extend_from_slice(&new_symbols);

                    decode_text_region(
                        huffman,
                        refinement,
                        current_width as usize,
                        current_height as usize,
                        0,
                        number_of_instances as usize,
                        1,
                        &all_symbols,
                        symbol_code_length,
                        false, // transposed = 0
                        0,     // ds offset
                        1,     // top left 7.4.3.1.1
                        0,     // OR operator
                        // TODO: Why unreachable?
                        huffman_tables.map(|_| unreachable!("no text region huffman tables")),
                        refinement_template_index,
                        refinement_at,
                        decoding_context,
                        0,
                        // TODO: Figure out how to align this
                        huffman_input,
                    )?
                } else {
                    let symbol_id = decoding_context.decode_iaid(symbol_code_length) as usize;
                    let rdx = decoding_context
                        .decode_integer("IARDX") // 6.4.11.3
                        .ok_or_else(|| Jbig2Error::new("Failed to decode IARDX"))?;
                    let rdy = decoding_context
                        .decode_integer("IARDY") // 6.4.11.4
                        .ok_or_else(|| Jbig2Error::new("Failed to decode IARDY"))?;

                    let symbol = if symbol_id < symbols.len() {
                        &symbols[symbol_id]
                    } else {
                        &new_symbols[symbol_id - symbols.len()]
                    };

                    decode_refinement(
                        current_width as usize,
                        current_height as usize,
                        refinement_template_index,
                        symbol,
                        rdx,
                        rdy,
                        false,
                        refinement_at,
                        decoding_context,
                    )?
                };
                new_symbols.push(bitmap);
            } else if huffman {
                // Store only symbol width and decode a collective bitmap when the height class is done.
                symbol_widths.push(current_width);
            } else {
                // 6.5.8.1 Direct-coded symbol bitmap
                let bitmap = decode_bitmap(
                    false,
                    current_width as usize,
                    current_height as usize,
                    template_index,
                    false,
                    None,
                    at,
                    decoding_context,
                )?;
                new_symbols.push(bitmap);
            }
        }

        if huffman && !refinement {
            let huffman_input = huffman_input.clone().unwrap();

            // 6.5.9 Height class collective bitmap
            let bitmap_size = huffman_tables
                .as_ref()
                .unwrap()
                .bitmap_size_table
                .as_ref()
                .ok_or_else(|| Jbig2Error::new("Bitmap size table required"))?
                .decode(huffman_input)?
                .ok_or_else(|| Jbig2Error::new("Got OOB for bitmap size"))?;

            huffman_input.byte_align();

            let collective_bitmap = if bitmap_size == 0 {
                // Uncompressed collective bitmap
                read_uncompressed_bitmap(
                    huffman_input,
                    total_width as usize,
                    current_height as usize,
                )?
            } else {
                // MMR collective bitmap
                let mut input = huffman_input.0.borrow_mut();
                let original_end = input.end;
                let bitmap_end = input.position + bitmap_size as usize;
                input.end = bitmap_end;
                std::mem::drop(input);

                let result = decode_mmr_bitmap(
                    &huffman_input,
                    total_width as usize,
                    current_height as usize,
                    false,
                );

                let mut input = huffman_input.0.borrow_mut();
                input.end = original_end;
                input.position = bitmap_end;

                result?
            };

            let number_of_symbols_decoded = symbol_widths.len();
            if first_symbol == number_of_symbols_decoded - 1 {
                // collectiveBitmap is a single symbol.
                new_symbols.push(collective_bitmap);
            } else {
                // Divide collectiveBitmap into symbols.
                let mut x_min = 0;
                for i in first_symbol..number_of_symbols_decoded {
                    let bitmap_width = symbol_widths[i] as usize;
                    let x_max = x_min + bitmap_width;
                    let mut symbol_bitmap = Vec::new();
                    for y in 0..(current_height as usize) {
                        symbol_bitmap.push(collective_bitmap[y][x_min..x_max].to_vec());
                    }
                    new_symbols.push(symbol_bitmap);
                    x_min = x_max;
                }
            }
        }
    }

    // 6.5.10 Exported symbols
    let mut exported_symbols = Vec::new();
    let mut flags = Vec::new();
    let mut current_flag = false;
    let total_symbols_length = symbols.len() + number_of_new_symbols;

    while flags.len() < total_symbols_length {
        let run_length = if huffman {
            let huffman_input = huffman_input.clone().unwrap();
            let res = table_b1
                .as_ref()
                .unwrap()
                .decode(huffman_input)?
                .ok_or_else(|| Jbig2Error::new("Got OOB for run length"))?;

            res
        } else {
            decoding_context
                .decode_integer("IAEX")
                .ok_or_else(|| Jbig2Error::new("Failed to decode IAEX"))?
        };

        for _ in 0..run_length {
            flags.push(current_flag);
        }
        current_flag = !current_flag;
    }

    // println!("flags: {:?}", flags);
    // Export symbols based on flags
    for (i, &flag) in flags.iter().enumerate().take(symbols.len()) {
        if flag {
            exported_symbols.push(symbols[i].clone());
        }
    }

    for (j, symbol) in new_symbols.iter().enumerate() {
        let i = symbols.len() + j;
        if i < flags.len() && flags[i] {
            exported_symbols.push(symbol.clone());
        }
    }

    Ok(exported_symbols)
}
