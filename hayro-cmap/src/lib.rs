/*!
A CMap parser.

This crate provides a parser for CMap files, which are used in PDF to map
character codes to Unicode values.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;
