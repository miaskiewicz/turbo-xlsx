//! A tiny, dependency-free, deterministic ZIP writer for OPC packaging.
//!
//! XLSX is an OPC package: a ZIP of XML parts. Entries are written **STORED**
//! (uncompressed, method 0) — Excel and every conformant reader accept a stored
//! OPC zip, and storing keeps the writer free of a DEFLATE dependency and fully
//! deterministic (a fixed 1980-01-01 timestamp, no zlib version bytes), so the
//! same workbook always produces byte-identical output. DEFLATE is a future
//! size optimization, not a correctness requirement.

/// One archive member: its OPC part name and raw bytes.
pub struct Part {
    pub name: String,
    pub data: Vec<u8>,
}

/// Build a complete STORED ZIP archive from `parts`, in the order given.
pub fn build(parts: &[Part]) -> Vec<u8> {
    // Reserve up front: each entry is a 30-byte local header + name + data, plus a
    // 46-byte central record + name, plus the 22-byte EOCD. Avoids reallocating a
    // tens-of-MB buffer log2(size) times on a large export.
    let payload: usize = parts
        .iter()
        .map(|p| p.data.len() + 2 * p.name.len() + 76)
        .sum();
    let mut out = Vec::with_capacity(payload + 22);
    let mut central = Vec::with_capacity(parts.iter().map(|p| p.name.len() + 46).sum());
    let mut count: u16 = 0;
    for part in parts {
        let offset = out.len() as u32;
        let crc = crc32(&part.data);
        write_local_header(&mut out, part, crc);
        out.extend_from_slice(&part.data);
        write_central_record(&mut central, part, crc, offset);
        count += 1;
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    write_eocd(&mut out, count, central_size, central_offset);
    out
}

/// Write a local file header followed by nothing else (caller appends data).
fn write_local_header(out: &mut Vec<u8>, part: &Part, crc: u32) {
    let size = part.data.len() as u32;
    push_u32(out, 0x0403_4b50); // local file header signature
    push_u16(out, 20); // version needed
    push_u16(out, 0); // flags
    push_u16(out, 0); // method 0 = stored
    push_u16(out, 0); // mod time (fixed)
    push_u16(out, 0x0021); // mod date = 1980-01-01
    push_u32(out, crc);
    push_u32(out, size); // compressed size == size (stored)
    push_u32(out, size); // uncompressed size
    push_u16(out, part.name.len() as u16);
    push_u16(out, 0); // extra length
    out.extend_from_slice(part.name.as_bytes());
}

/// Write a central-directory record for a part.
fn write_central_record(out: &mut Vec<u8>, part: &Part, crc: u32, offset: u32) {
    let size = part.data.len() as u32;
    push_u32(out, 0x0201_4b50); // central directory signature
    push_u16(out, 20); // version made by
    push_u16(out, 20); // version needed
    push_u16(out, 0); // flags
    push_u16(out, 0); // method
    push_u16(out, 0); // mod time
    push_u16(out, 0x0021); // mod date
    push_u32(out, crc);
    push_u32(out, size);
    push_u32(out, size);
    push_u16(out, part.name.len() as u16);
    push_u16(out, 0); // extra length
    push_u16(out, 0); // comment length
    push_u16(out, 0); // disk number start
    push_u16(out, 0); // internal attrs
    push_u32(out, 0); // external attrs
    push_u32(out, offset);
    out.extend_from_slice(part.name.as_bytes());
}

/// Write the end-of-central-directory record.
fn write_eocd(out: &mut Vec<u8>, count: u16, central_size: u32, central_offset: u32) {
    push_u32(out, 0x0605_4b50); // EOCD signature
    push_u16(out, 0); // disk number
    push_u16(out, 0); // disk with central dir
    push_u16(out, count); // entries on this disk
    push_u16(out, count); // total entries
    push_u32(out, central_size);
    push_u32(out, central_offset);
    push_u16(out, 0); // comment length
}

/// Append a little-endian `u16`.
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Append a little-endian `u32`.
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The slice-by-8 CRC-32 lookup tables (polynomial 0xEDB88320), built once at
/// runtime on first use. Eight 256-entry tables let us fold 8 input bytes per
/// iteration instead of one bit at a time — the dominant cost when checksumming
/// tens of MB of worksheet XML for the zip.
fn crc_tables() -> &'static [[u32; 256]; 8] {
    use std::sync::OnceLock;
    static TABLES: OnceLock<[[u32; 256]; 8]> = OnceLock::new();
    TABLES.get_or_init(build_crc_tables)
}

/// Populate the slice-by-8 tables. Table 0 is the classic bit-reflected CRC
/// table; tables 1..8 fold successive byte positions.
#[allow(clippy::needless_range_loop)]
fn build_crc_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    for n in 0..256 {
        let mut crc = n as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
        t[0][n] = crc;
    }
    for n in 0..256 {
        let mut crc = t[0][n];
        for k in 1..8 {
            crc = t[0][(crc & 0xff) as usize] ^ (crc >> 8);
            t[k][n] = crc;
        }
    }
    t
}

/// Compute the IEEE CRC-32 of `data` (polynomial 0xEDB88320), the checksum ZIP
/// stores per entry. Slice-by-8: 8 bytes folded per step, then a byte-wise tail.
pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_tables();
    let mut crc: u32 = 0xFFFF_FFFF;
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        crc ^= u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        crc = t[7][(crc & 0xff) as usize]
            ^ t[6][((crc >> 8) & 0xff) as usize]
            ^ t[5][((crc >> 16) & 0xff) as usize]
            ^ t[4][(crc >> 24) as usize]
            ^ t[3][c[4] as usize]
            ^ t[2][c[5] as usize]
            ^ t[1][c[6] as usize]
            ^ t[0][c[7] as usize];
    }
    for &b in chunks.remainder() {
        crc = t[0][((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        // Standard IEEE CRC-32 of "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn builds_parsable_archive() {
        let parts = vec![
            Part {
                name: "a.txt".to_string(),
                data: b"hello".to_vec(),
            },
            Part {
                name: "b.txt".to_string(),
                data: b"world!!".to_vec(),
            },
        ];
        let zip = build(&parts);
        // Local header + EOCD signatures present.
        assert_eq!(&zip[0..4], &0x0403_4b50u32.to_le_bytes());
        assert_eq!(
            &zip[zip.len() - 22..zip.len() - 18],
            &0x0605_4b50u32.to_le_bytes()
        );
        // Entry count in EOCD is 2.
        let count = u16::from_le_bytes([zip[zip.len() - 14], zip[zip.len() - 13]]);
        assert_eq!(count, 2);
        // The stored payloads appear verbatim.
        assert!(zip.windows(5).any(|w| w == b"hello"));
        assert!(zip.windows(7).any(|w| w == b"world!!"));
    }
}
