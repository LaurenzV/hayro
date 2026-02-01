/*!
A lightweight PostScript parser and interpreter.

This crate provides parsing and interpretation of PostScript programs,
focused on the subset commonly used in PDF documents (Type 1 fonts,
CFF/Type 2 charstrings, etc.).

The crate is `no_std` compatible but requires an allocator to be available.

# Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;
