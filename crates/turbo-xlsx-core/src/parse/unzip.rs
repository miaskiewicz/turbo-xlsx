//! A minimal, read-only ZIP reader for OPC packages — STORED + DEFLATE entries.
//!
//! Authoritative: it reads the central directory (entry sizes + offsets), not the
//! streamed local headers, so files written with data descriptors still parse.

use super::inflate::inflate;
use super::ParseError;

type PResult<T> = Result<T, ParseError>;

fn bad() -> ParseError {
    ParseError::Zip
}

/// One archive member: its part name and the (decompressed) bytes.
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Read every member of the zip `bytes`.
pub fn read_zip(bytes: &[u8]) -> PResult<Vec<Entry>> {
    let eocd = find_eocd(bytes)?;
    let count = u16le(bytes, eocd + 10)?;
    let start = u32le(bytes, eocd + 16)?;
    read_entries(bytes, count, start)
}

/// Read `count` central-directory entries starting at `pos`.
fn read_entries(bytes: &[u8], count: usize, mut pos: usize) -> PResult<Vec<Entry>> {
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, next) = read_central_entry(bytes, pos)?;
        entries.push(entry);
        pos = next;
    }
    Ok(entries)
}

/// Find the end-of-central-directory record by scanning back for its signature.
fn find_eocd(bytes: &[u8]) -> PResult<usize> {
    if bytes.len() < 22 {
        return Err(bad());
    }
    let sig = [0x50u8, 0x4b, 0x05, 0x06];
    for p in (0..=bytes.len() - 22).rev() {
        if bytes[p..p + 4] == sig {
            return Ok(p);
        }
    }
    Err(bad())
}

/// One central-directory record's fixed fields.
struct CdFields {
    method: usize,
    comp_size: usize,
    name_len: usize,
    extra_len: usize,
    comment_len: usize,
    local_off: usize,
    name: String,
}

/// Read one central-directory entry at `pos`, returning it + the next position.
fn read_central_entry(bytes: &[u8], pos: usize) -> PResult<(Entry, usize)> {
    if u32le(bytes, pos)? != 0x0201_4b50 {
        return Err(bad());
    }
    let cd = read_cd_fields(bytes, pos)?;
    let data = read_member_data(bytes, cd.local_off, cd.method, cd.comp_size)?;
    let next = pos + 46 + cd.name_len + cd.extra_len + cd.comment_len;
    Ok((
        Entry {
            name: cd.name,
            data,
        },
        next,
    ))
}

/// Read the fixed 46-byte central-directory header + the entry name.
fn read_cd_fields(bytes: &[u8], pos: usize) -> PResult<CdFields> {
    let h = slice(bytes, pos, 46)?;
    let name_len = u16(h, 28);
    Ok(CdFields {
        method: u16(h, 10),
        comp_size: u32(h, 20),
        name_len,
        extra_len: u16(h, 30),
        comment_len: u16(h, 32),
        local_off: u32(h, 42),
        name: read_str(bytes, pos + 46, name_len)?,
    })
}

/// Read + decompress the member data referenced by a central-directory entry.
fn read_member_data(
    bytes: &[u8],
    local_off: usize,
    method: usize,
    comp_size: usize,
) -> PResult<Vec<u8>> {
    let lh = slice(bytes, local_off, 30)?;
    if u32(lh, 0) != 0x0403_4b50 {
        return Err(bad());
    }
    let start = local_off + 30 + u16(lh, 26) + u16(lh, 28);
    let raw = slice(bytes, start, comp_size)?;
    decompress(raw, method)
}

/// STORED (method 0) → copy; DEFLATE (method 8) → inflate; else error.
fn decompress(raw: &[u8], method: usize) -> PResult<Vec<u8>> {
    match method {
        0 => Ok(raw.to_vec()),
        8 => inflate(raw),
        _ => Err(bad()),
    }
}

/// A bounds-checked sub-slice.
fn slice(bytes: &[u8], start: usize, len: usize) -> PResult<&[u8]> {
    bytes.get(start..start + len).ok_or_else(bad)
}

/// A bounds-checked UTF-8 (lossy) string of `len` bytes at `off`.
fn read_str(bytes: &[u8], off: usize, len: usize) -> PResult<String> {
    Ok(String::from_utf8_lossy(slice(bytes, off, len)?).into_owned())
}

/// A bounds-checked little-endian `u16` at `off`.
fn u16le(bytes: &[u8], off: usize) -> PResult<usize> {
    Ok(u16(slice(bytes, off, 2)?, 0))
}

/// A bounds-checked little-endian `u32` at `off`.
fn u32le(bytes: &[u8], off: usize) -> PResult<usize> {
    Ok(u32(slice(bytes, off, 4)?, 0))
}

/// A little-endian `u16` from a header slice known to be long enough.
fn u16(h: &[u8], o: usize) -> usize {
    h[o] as usize | (h[o + 1] as usize) << 8
}

/// A little-endian `u32` from a header slice known to be long enough.
fn u32(h: &[u8], o: usize) -> usize {
    h[o] as usize | (h[o + 1] as usize) << 8 | (h[o + 2] as usize) << 16 | (h[o + 3] as usize) << 24
}
