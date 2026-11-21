//! Static Huffman coder used for packet compression.
//! C++: `src/engine/shared/huffman.{h,cpp}` (`CHuffman`).

use std::sync::OnceLock;

const EOF_SYMBOL: usize = 256;
const MAX_SYMBOLS: usize = EOF_SYMBOL + 1;
const MAX_NODES: usize = MAX_SYMBOLS * 2 - 1;
const LUT_BITS: u32 = 10;
const LUT_SIZE: usize = 1 << LUT_BITS;
const LUT_MASK: u32 = LUT_SIZE as u32 - 1;
const NO_LEAF: u16 = 0xffff;

/// C++: `CHuffman::ms_aFreqTable` (huffman.cpp:11).
#[rustfmt::skip]
const FREQ_TABLE: [u32; MAX_SYMBOLS] = [
    1 << 30, 4545, 2657, 431, 1950, 919, 444, 482, 2244, 617, 838, 542, 715, 1814, 304, 240, 754, 212, 647, 186,
    283, 131, 146, 166, 543, 164, 167, 136, 179, 859, 363, 113, 157, 154, 204, 108, 137, 180, 202, 176,
    872, 404, 168, 134, 151, 111, 113, 109, 120, 126, 129, 100, 41, 20, 16, 22, 18, 18, 17, 19,
    16, 37, 13, 21, 362, 166, 99, 78, 95, 88, 81, 70, 83, 284, 91, 187, 77, 68, 52, 68,
    59, 66, 61, 638, 71, 157, 50, 46, 69, 43, 11, 24, 13, 19, 10, 12, 12, 20, 14, 9,
    20, 20, 10, 10, 15, 15, 12, 12, 7, 19, 15, 14, 13, 18, 35, 19, 17, 14, 8, 5,
    15, 17, 9, 15, 14, 18, 8, 10, 2173, 134, 157, 68, 188, 60, 170, 60, 194, 62, 175, 71,
    148, 67, 167, 78, 211, 67, 156, 69, 1674, 90, 174, 53, 147, 89, 181, 51, 174, 63, 163, 80,
    167, 94, 128, 122, 223, 153, 218, 77, 200, 110, 190, 73, 174, 69, 145, 66, 277, 143, 141, 60,
    136, 53, 180, 57, 142, 57, 158, 61, 166, 112, 152, 92, 26, 22, 21, 28, 20, 26, 30, 21,
    32, 27, 20, 17, 23, 21, 30, 22, 22, 21, 27, 25, 17, 27, 23, 18, 39, 26, 15, 21,
    12, 18, 18, 27, 20, 18, 15, 19, 11, 17, 33, 12, 18, 15, 19, 18, 16, 26, 17, 18,
    9, 10, 25, 22, 22, 17, 20, 16, 6, 16, 15, 20, 14, 18, 24, 335, 1,
];

/// C++: `CHuffman::CNode`. `num_bits != 0` marks a leaf.
#[derive(Clone, Copy, Default)]
struct Node {
    bits: u32,
    num_bits: u32,
    leaves: [u16; 2],
    symbol: u8,
}

struct Huffman {
    nodes: [Node; MAX_NODES],
    decode_lut: [u16; LUT_SIZE],
}

fn huffman() -> &'static Huffman {
    static HUFFMAN: OnceLock<Huffman> = OnceLock::new();
    HUFFMAN.get_or_init(Huffman::new)
}

impl Huffman {
    /// C++: `CHuffman::Init` + `ConstructTree` (huffman.cpp:52, 95).
    fn new() -> Self {
        let mut nodes = [Node::default(); MAX_NODES];
        // (node id, frequency), C++: `CHuffmanConstructNode`
        let mut left: Vec<(u16, i32)> = Vec::with_capacity(MAX_SYMBOLS);
        for (i, &freq) in FREQ_TABLE.iter().enumerate() {
            nodes[i] = Node {
                bits: 0,
                num_bits: 0xFFFF_FFFF,
                leaves: [NO_LEAF; 2],
                symbol: i as u8, // EOF (256) truncates to 0, as in C++
            };
            left.push((i as u16, freq as i32));
        }

        let mut num_nodes = MAX_SYMBOLS;
        while left.len() > 1 {
            // C++ `std::stable_sort` by frequency descending; `sort_by_key` is stable too.
            left.sort_by_key(|&(_, freq)| std::cmp::Reverse(freq));
            let n = left.len();
            nodes[num_nodes] = Node {
                num_bits: 0,
                leaves: [left[n - 1].0, left[n - 2].0],
                ..Node::default()
            };
            left[n - 2] = (num_nodes as u16, left[n - 1].1 + left[n - 2].1);
            left.pop();
            num_nodes += 1;
        }
        let start = (num_nodes - 1) as u16;
        set_bits_recursive(&mut nodes, start, 0, 0);

        // decode LUT: follow up to LUT_BITS bits from the root
        let mut decode_lut = [0u16; LUT_SIZE];
        for (i, entry) in decode_lut.iter_mut().enumerate() {
            let mut bits = i;
            let mut node = start;
            for _ in 0..LUT_BITS {
                node = nodes[node as usize].leaves[bits & 1];
                bits >>= 1;
                if nodes[node as usize].num_bits != 0 {
                    break;
                }
            }
            *entry = node;
        }

        Self { nodes, decode_lut }
    }
}

/// C++: `CHuffman::SetBitsRecursive` (huffman.cpp:38).
fn set_bits_recursive(nodes: &mut [Node; MAX_NODES], node: u16, bits: u32, depth: u32) {
    let n = nodes[node as usize];
    if n.leaves[1] != NO_LEAF {
        set_bits_recursive(nodes, n.leaves[1], bits | (1 << depth), depth + 1);
    }
    if n.leaves[0] != NO_LEAF {
        set_bits_recursive(nodes, n.leaves[0], bits, depth + 1);
    }
    if n.num_bits != 0 {
        nodes[node as usize].bits = bits;
        nodes[node as usize].num_bits = depth;
    }
}

/// Compresses `input` into `output`, returning the compressed size, or `None` if
/// `output` is too small. C++: `CHuffman::Compress` (huffman.cpp:130). The C++
/// fast path that stores 8 bytes at once writes the same bytes as this loop.
pub fn compress(input: &[u8], output: &mut [u8]) -> Option<usize> {
    let h = huffman();
    let mut bits: u64 = 0;
    let mut bit_count: u32 = 0;
    let mut dst = 0;

    let mut emit = |symbol: usize| -> Option<()> {
        let node = &h.nodes[symbol];
        bits |= (node.bits as u64) << bit_count;
        bit_count += node.num_bits;
        while bit_count >= 8 {
            *output.get_mut(dst)? = bits as u8;
            dst += 1;
            bits >>= 8;
            bit_count -= 8;
        }
        Some(())
    };
    for &byte in input {
        emit(byte as usize)?;
    }
    emit(EOF_SYMBOL)?;

    // write out the last bits
    if bit_count != 0 {
        *output.get_mut(dst)? = bits as u8;
        dst += 1;
    }
    Some(dst)
}

/// Decompresses `input` into `output`, returning the size, or `None` on malformed
/// input or if `output` is too small. C++: `CHuffman::Decompress` (huffman.cpp:225).
///
/// Quirk kept on purpose: when a tree walk starts with 0 bits left, the C++
/// `unsigned` bit counter wraps to `0xFFFFFFFF` and decoding continues on zero
/// bits; the wrapping arithmetic below reproduces that exactly.
pub fn decompress(input: &[u8], output: &mut [u8]) -> Option<usize> {
    let h = huffman();
    let mut src = input.iter();
    let mut dst = 0;
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;

    loop {
        // C++ looks the LUT up before refilling when it already has LUT_BITS bits;
        // the refill only adds bits above those, so looking up afterwards is identical.
        while bit_count < 24 {
            let Some(&byte) = src.next() else { break };
            bits |= (byte as u32) << bit_count;
            bit_count += 8;
        }
        let mut node = h.decode_lut[(bits & LUT_MASK) as usize];

        let num_bits = h.nodes[node as usize].num_bits;
        if num_bits != 0 {
            // hit a symbol directly
            if bit_count < num_bits {
                return None;
            }
            bits >>= num_bits;
            bit_count -= num_bits;
        } else {
            if bit_count < LUT_BITS {
                return None;
            }
            bits >>= LUT_BITS;
            bit_count -= LUT_BITS;
            // walk the tree bit by bit
            loop {
                node = h.nodes[node as usize].leaves[(bits & 1) as usize];
                bit_count = bit_count.wrapping_sub(1);
                bits >>= 1;
                if h.nodes[node as usize].num_bits != 0 {
                    break;
                }
                if bit_count == 0 {
                    return None;
                }
            }
        }

        if node as usize == EOF_SYMBOL {
            return Some(dst);
        }
        *output.get_mut(dst)? = h.nodes[node as usize].symbol;
        dst += 1;
    }
}
