//! Minimal XML escaping for the OOXML parts we emit.
//!
//! SpreadsheetML is plain XML; the only characters that must be escaped in text
//! and attribute values are `& < > " '`. We additionally strip the handful of
//! control characters that are illegal in XML 1.0 (everything below U+0020 except
//! tab/newline/carriage-return) so a stray control byte in caller data can never
//! produce a malformed — and therefore unopenable — workbook.

/// Escape `s` for use as XML element text or an attribute value, dropping XML-1.0
/// illegal control characters.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(&mut out, s);
    out
}

/// Escape `s` directly onto `out` (no intermediate allocation) — the hot path for
/// per-cell inline-string emission.
pub fn escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        push_escaped(out, ch);
    }
}

/// Append `ch` to `out`, replacing the five XML metacharacters with entities and
/// skipping characters that are not legal in XML 1.0.
fn push_escaped(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&apos;"),
        _ if is_legal(ch) => out.push(ch),
        _ => {}
    }
}

/// Whether `ch` is a character XML 1.0 permits in content. Tab, newline and
/// carriage return are the only sub-U+0020 characters allowed.
fn is_legal(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r') || ch >= ' '
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_metacharacters() {
        assert_eq!(escape("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
    }

    #[test]
    fn keeps_legal_whitespace_and_text() {
        assert_eq!(escape("x\t\n\ry"), "x\t\n\ry");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn drops_illegal_control_characters() {
        assert_eq!(escape("a\u{0001}\u{0007}b"), "ab");
    }
}
