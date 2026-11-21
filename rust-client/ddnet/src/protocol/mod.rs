//! Wire format. Pure functions over bytes: no I/O, no clock, no randomness.

pub mod huffman;
pub mod packer;
pub mod packet;
pub mod varint;
