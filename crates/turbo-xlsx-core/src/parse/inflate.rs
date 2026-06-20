//! A small, dependency-free DEFLATE (RFC 1951) decompressor.
//!
//! turbo-xlsx writes STORED (uncompressed) zips, but files produced by Excel /
//! LibreOffice / other writers DEFLATE their parts, so reading real `.xlsx`
//! needs an inflater. This is a safe-Rust port of the canonical *puff* algorithm
//! (zlib's reference decoder): a bit reader, canonical Huffman decode by the
//! count/offset method, and the stored / fixed / dynamic block paths. Decomposed
//! into small functions to satisfy the repo's cyclomatic-complexity gate.

use super::ParseError;

type PResult<T> = Result<T, ParseError>;

fn bad() -> ParseError {
    ParseError::Deflate
}

/// LSB-first bit reader over the compressed bytes.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bitbuf: u32,
    bitcnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
        }
    }

    /// Read a single bit (refilling a byte when the buffer is empty).
    fn take_bit(&mut self) -> PResult<u32> {
        if self.bitcnt == 0 {
            self.bitbuf = *self.data.get(self.pos).ok_or_else(bad)? as u32;
            self.pos += 1;
            self.bitcnt = 8;
        }
        let bit = self.bitbuf & 1;
        self.bitbuf >>= 1;
        self.bitcnt -= 1;
        Ok(bit)
    }

    /// Read `n` bits, LSB first.
    fn take_bits(&mut self, n: u32) -> PResult<u32> {
        let mut value = 0u32;
        for i in 0..n {
            value |= self.take_bit()? << i;
        }
        Ok(value)
    }

    /// Discard the rest of the current byte (for a stored block).
    fn align(&mut self) {
        self.bitcnt = 0;
        self.bitbuf = 0;
    }

    /// Read one whole byte (only valid right after `align`).
    fn take_byte(&mut self) -> PResult<u8> {
        let byte = *self.data.get(self.pos).ok_or_else(bad)?;
        self.pos += 1;
        Ok(byte)
    }

    /// Read a little-endian `u16` (after `align`).
    fn take_u16(&mut self) -> PResult<usize> {
        let lo = self.take_byte()? as usize;
        let hi = self.take_byte()? as usize;
        Ok(lo | (hi << 8))
    }
}

/// A canonical Huffman table: `counts[len]` codes of each length, `symbols`
/// sorted by (length, symbol) — the decode shape from puff.
struct Huff {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huff {
    /// Build a table from per-symbol code lengths (0 = unused).
    fn build(lengths: &[u8]) -> Huff {
        let mut counts = [0u16; 16];
        for &len in lengths {
            counts[len as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for len in 1..16 {
            offsets[len] = offsets[len - 1] + counts[len - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &len) in lengths.iter().enumerate() {
            place_symbol(&mut symbols, &mut offsets, sym, len);
        }
        Huff { counts, symbols }
    }

    /// Decode one symbol by the canonical count/offset method.
    fn decode(&self, r: &mut BitReader) -> PResult<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.take_bit()? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(bad())
    }
}

/// Place one non-zero-length symbol into its canonical slot.
fn place_symbol(symbols: &mut [u16], offsets: &mut [u16; 16], sym: usize, len: u8) {
    if len != 0 {
        let slot = offsets[len as usize] as usize;
        symbols[slot] = sym as u16;
        offsets[len as usize] += 1;
    }
}

// Length/distance base values + extra-bit counts (RFC 1951 §3.2.5).
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Decompress a complete DEFLATE stream.
pub fn inflate(data: &[u8]) -> PResult<Vec<u8>> {
    let mut r = BitReader::new(data);
    let mut out = Vec::new();
    while !inflate_one_block(&mut r, &mut out)? {}
    Ok(out)
}

/// Decode one block; returns whether it was the final block.
fn inflate_one_block(r: &mut BitReader, out: &mut Vec<u8>) -> PResult<bool> {
    let bfinal = r.take_bit()? == 1;
    let btype = r.take_bits(2)?;
    inflate_typed_block(r, out, btype)?;
    Ok(bfinal)
}

/// Decode a block of the given BTYPE.
fn inflate_typed_block(r: &mut BitReader, out: &mut Vec<u8>, btype: u32) -> PResult<()> {
    match btype {
        0 => stored_block(r, out),
        1 => {
            let (lit, dist) = fixed_huffs();
            huffman_block(r, out, &lit, &dist)
        }
        2 => {
            let (lit, dist) = read_dynamic_huffs(r)?;
            huffman_block(r, out, &lit, &dist)
        }
        _ => Err(bad()),
    }
}

/// Copy a stored (uncompressed) block.
fn stored_block(r: &mut BitReader, out: &mut Vec<u8>) -> PResult<()> {
    r.align();
    let len = r.take_u16()?;
    let _nlen = r.take_u16()?;
    for _ in 0..len {
        out.push(r.take_byte()?);
    }
    Ok(())
}

/// The fixed (predefined) literal/length + distance tables.
fn fixed_huffs() -> (Huff, Huff) {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = fixed_lit_len(i);
    }
    (Huff::build(&lit), Huff::build(&[5u8; 30]))
}

/// The fixed literal/length code length for symbol `i` (RFC 1951 §3.2.6).
fn fixed_lit_len(i: usize) -> u8 {
    match i {
        0..=143 => 8,
        144..=255 => 9,
        256..=279 => 7,
        _ => 8,
    }
}

/// Read the dynamic literal/length + distance tables for a block.
fn read_dynamic_huffs(r: &mut BitReader) -> PResult<(Huff, Huff)> {
    let (clen_huff, hlit, hdist) = read_clen_huff(r)?;
    let all = read_code_lengths(r, &clen_huff, hlit + hdist)?;
    Ok((Huff::build(&all[..hlit]), Huff::build(&all[hlit..])))
}

/// Read the header counts + the code-length-code Huffman table.
fn read_clen_huff(r: &mut BitReader) -> PResult<(Huff, usize, usize)> {
    let hlit = r.take_bits(5)? as usize + 257;
    let hdist = r.take_bits(5)? as usize + 1;
    let hclen = r.take_bits(4)? as usize + 4;
    let lengths = read_clen_lengths(r, hclen)?;
    Ok((Huff::build(&lengths), hlit, hdist))
}

/// Read the `hclen` code-length code lengths, in their permuted order.
fn read_clen_lengths(r: &mut BitReader, hclen: usize) -> PResult<[u8; 19]> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let mut lengths = [0u8; 19];
    for i in 0..hclen {
        lengths[ORDER[i]] = r.take_bits(3)? as u8;
    }
    Ok(lengths)
}

/// Decode `total` literal/length + distance code lengths (incl. repeat codes).
fn read_code_lengths(r: &mut BitReader, h: &Huff, total: usize) -> PResult<Vec<u8>> {
    let mut out = Vec::with_capacity(total);
    while out.len() < total {
        let sym = h.decode(r)?;
        apply_code_length(r, &mut out, sym)?;
    }
    Ok(out)
}

/// Apply one code-length symbol: a literal length, or a repeat (16/17/18).
fn apply_code_length(r: &mut BitReader, out: &mut Vec<u8>, sym: u16) -> PResult<()> {
    match sym {
        0..=15 => out.push(sym as u8),
        16 => repeat_prev(r, out)?,
        17 => repeat_zero(r, out, 3, 3)?,
        18 => repeat_zero(r, out, 11, 7)?,
        _ => return Err(bad()),
    }
    Ok(())
}

/// Code 16: repeat the previous code length 3–6 times.
fn repeat_prev(r: &mut BitReader, out: &mut Vec<u8>) -> PResult<()> {
    let prev = *out.last().ok_or_else(bad)?;
    let count = 3 + r.take_bits(2)? as usize;
    for _ in 0..count {
        out.push(prev);
    }
    Ok(())
}

/// Codes 17/18: repeat a zero code length `base + extra-bits` times.
fn repeat_zero(r: &mut BitReader, out: &mut Vec<u8>, base: usize, extra: u32) -> PResult<()> {
    let count = base + r.take_bits(extra)? as usize;
    for _ in 0..count {
        out.push(0);
    }
    Ok(())
}

/// Decode a Huffman-coded block (literals + length/distance back-references).
fn huffman_block(r: &mut BitReader, out: &mut Vec<u8>, lit: &Huff, dist: &Huff) -> PResult<()> {
    loop {
        let sym = lit.decode(r)?;
        match sym {
            256 => return Ok(()),
            0..=255 => out.push(sym as u8),
            _ => emit_match(r, out, sym, dist)?,
        }
    }
}

/// Emit a length/distance match: copy `len` bytes from `dist` back in `out`.
fn emit_match(r: &mut BitReader, out: &mut Vec<u8>, sym: u16, dist: &Huff) -> PResult<()> {
    let len = read_length(r, sym)?;
    let dsym = dist.decode(r)?;
    let distance = read_distance(r, dsym)?;
    copy_back(out, len, distance)
}

/// Resolve a length code (257..285) to a length, reading its extra bits.
fn read_length(r: &mut BitReader, sym: u16) -> PResult<usize> {
    let i = (sym as usize)
        .checked_sub(257)
        .filter(|&i| i < 29)
        .ok_or_else(bad)?;
    Ok(LEN_BASE[i] as usize + r.take_bits(LEN_EXTRA[i])? as usize)
}

/// Resolve a distance code (0..29) to a distance, reading its extra bits.
fn read_distance(r: &mut BitReader, sym: u16) -> PResult<usize> {
    let i = sym as usize;
    if i >= 30 {
        return Err(bad());
    }
    Ok(DIST_BASE[i] as usize + r.take_bits(DIST_EXTRA[i])? as usize)
}

/// Copy `len` bytes from `dist` back in the output (LZ77, may self-overlap).
fn copy_back(out: &mut Vec<u8>, len: usize, dist: usize) -> PResult<()> {
    if dist == 0 || dist > out.len() {
        return Err(bad());
    }
    let start = out.len() - dist;
    for i in 0..len {
        let byte = out[start + i];
        out.push(byte);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_block_round_trips() {
        // BFINAL=1,BTYPE=00 (0x01), then aligned LEN=2, NLEN, "Hi".
        let data = [0x01u8, 0x02, 0x00, 0xfd, 0xff, b'H', b'i'];
        assert_eq!(inflate(&data).unwrap(), b"Hi");
    }

    #[test]
    fn huffman_block_round_trips() {
        // raw DEFLATE (wbits=-15) of a sentence with repeated runs (back-refs).
        let deflate = [
            11u8, 201, 72, 85, 40, 44, 205, 76, 206, 86, 72, 42, 202, 47, 207, 83, 72, 203, 175,
            80, 200, 42, 205, 45, 40, 86, 200, 47, 75, 45, 82, 40, 1, 74, 231, 36, 86, 85, 42, 164,
            228, 167, 235, 41, 132, 224, 84, 156, 152, 158, 152, 153, 167, 144, 152, 151, 130, 206,
            210, 3, 0,
        ];
        let out = inflate(&deflate).unwrap();
        let expect = b"The quick brown fox jumps over the lazy dog. The quick brown fox jumps again and again and again.";
        assert_eq!(out, expect);
    }

    #[test]
    fn truncated_is_error() {
        assert!(inflate(&[]).is_err());
    }
}
