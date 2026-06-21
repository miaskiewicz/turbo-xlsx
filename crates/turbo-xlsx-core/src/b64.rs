//! A tiny, dependency-free base64 codec (standard alphabet, `=` padding).
//!
//! The writer carries embedded-image bytes base64-encoded in the JSON model, so
//! it needs to *decode* them into the raw `xl/media` part; the parser does the
//! inverse, *encoding* media bytes back into the round-trippable JSON. Both stay
//! in the core (the writer half is on the 100% coverage gate) and pull in no
//! crate, matching the writer's dependency-free zip/inflate ethos.

#[cfg(any(test, feature = "parse"))]
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Map one base64 character to its 6-bit value, or `None` if it is not part of
/// the alphabet (padding and whitespace are handled by the caller).
fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode a base64 string into bytes, ignoring ASCII whitespace and `=` padding.
/// Returns `None` on any other invalid character or a truncated final group (a
/// lone trailing sextet cannot encode a whole byte).
pub fn decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in input.as_bytes() {
        feed(&mut out, &mut acc, &mut bits, c)?;
    }
    remainder_ok(acc, bits).then_some(out)
}

/// Fold one input byte into the decoder: skip `=` padding / whitespace, reject an
/// out-of-alphabet byte (`None`), else shift in its sextet and flush whole bytes.
fn feed(out: &mut Vec<u8>, acc: &mut u32, bits: &mut u32, c: u8) -> Option<()> {
    if c == b'=' || c.is_ascii_whitespace() {
        return Some(());
    }
    *acc = (*acc << 6) | sextet(c)? as u32;
    *bits += 6;
    if *bits >= 8 {
        *bits -= 8;
        out.push((*acc >> *bits) as u8);
    }
    Some(())
}

/// A trailing partial sextet is valid only when its leftover bits are all zero;
/// a non-zero remainder means the input was truncated mid-byte.
fn remainder_ok(acc: u32, bits: u32) -> bool {
    bits == 0 || (acc & ((1 << bits) - 1)) == 0
}

/// Encode bytes into a base64 string (standard alphabet, `=`-padded). Used by
/// the parser (round-tripping media back to JSON) and the image-drawing tests;
/// gated out of the plain non-`parse` library build, where nothing encodes.
#[cfg(any(test, feature = "parse"))]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(pad_or(chunk.len() > 1, ALPHABET[(n >> 6) as usize & 63]));
        out.push(pad_or(chunk.len() > 2, ALPHABET[n as usize & 63]));
    }
    out
}

/// The encoded character when `keep`, else the `=` padding byte, as a `char`.
#[cfg(any(test, feature = "parse"))]
fn pad_or(keep: bool, c: u8) -> char {
    if keep {
        c as char
    } else {
        '='
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_chunk_remainders() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let enc = encode(s.as_bytes());
            assert_eq!(decode(&enc).unwrap(), s.as_bytes(), "round trip {s:?}");
        }
    }

    #[test]
    fn encodes_known_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn covers_full_alphabet() {
        let all: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
    }

    #[test]
    fn ignores_whitespace_and_padding() {
        assert_eq!(decode("Zm9v\n Ym Fy\t").unwrap(), b"foobar");
        assert_eq!(decode("Zg==").unwrap(), b"f");
    }

    #[test]
    fn rejects_invalid_char() {
        assert!(decode("Zm9v$").is_none());
    }

    #[test]
    fn rejects_truncated_group() {
        // A single sextet with non-zero low bits cannot encode a whole byte.
        assert!(decode("ZB").is_none());
    }

    #[test]
    fn accepts_zero_padded_remainder() {
        // "Zg" -> 'f' with two clean zero bits left over: valid (this is "Zg==").
        assert_eq!(decode("Zg").unwrap(), b"f");
    }
}
