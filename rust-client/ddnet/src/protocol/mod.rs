//! Wire format. Pure functions over bytes: no I/O, no clock, no randomness.

pub mod generated;
pub mod huffman;
pub mod netobj;
pub mod packer;
pub mod packet;
pub mod varint;
