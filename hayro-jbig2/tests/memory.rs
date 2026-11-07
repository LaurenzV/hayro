//! Streams decoded with a memory limit.

use crate::{check_memory_limit_exceeded, check_with_memory_limit};

#[test]
fn immediate_regions() {
    check_with_memory_limit("memory_immediate_regions", "truchet", 49152);
}

#[test]
fn intermediate_regions() {
    check_memory_limit_exceeded("memory_intermediate_regions", 49152);
}

#[test]
fn page() {
    check_memory_limit_exceeded("memory_page", 49152);
}

#[test]
fn halftone_grid() {
    check_memory_limit_exceeded("memory_halftone_grid", 49152);
}

#[test]
fn symbol_dictionary_large_symbol() {
    check_memory_limit_exceeded("memory_symbol_dictionary_large_symbol", 49152);
}

#[test]
fn symbol_dictionary_many_symbols() {
    check_memory_limit_exceeded("memory_symbol_dictionary_many_symbols", 8192);
}
