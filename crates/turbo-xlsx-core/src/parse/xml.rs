//! A tiny, dependency-free XML tokenizer — enough to read the well-formed OOXML
//! parts an `.xlsx` contains (`<tag attr="v">text</tag>`, self-closing tags,
//! entity-escaped text). Declarations / comments (`<? … ?>`, `<! … >`) are
//! skipped. Not a general XML parser; it trusts the OOXML it is fed.
//!
//! Tokens **borrow** from the input `&str`: tag/attribute names and unescaped
//! text are `&str` slices, and attribute/text values are [`Cow`] — owned only
//! when an `&entity;` actually has to be decoded. On a big sheet this turns the
//! per-cell run of `String` allocations into (mostly) pointer bumps.

use std::borrow::Cow;

/// How many attributes a tag keeps inline before spilling to the heap. OOXML
/// tags carry only a few (a worksheet cell has at most `r`, `s`, `t`), so four
/// inline slots cover the hot path and the per-tag `Vec` allocation disappears.
const INLINE_ATTRS: usize = 4;

/// A small attribute store: the first [`INLINE_ATTRS`] live in a stack array, the
/// rare wide tag spills the rest to a heap `Vec`. A hand-rolled mini "small vec"
/// (no extra dependency).
struct Attrs<'a> {
    inline: [Option<(&'a str, Cow<'a, str>)>; INLINE_ATTRS],
    len: usize,
    heap: Vec<(&'a str, Cow<'a, str>)>,
}

impl<'a> Attrs<'a> {
    /// An empty store (no allocation).
    fn new() -> Self {
        Attrs {
            inline: [None, None, None, None],
            len: 0,
            heap: Vec::new(),
        }
    }

    /// Append an attribute, staying inline until the slots are full.
    fn push(&mut self, key: &'a str, val: Cow<'a, str>) {
        if self.len < INLINE_ATTRS {
            self.inline[self.len] = Some((key, val));
            self.len += 1;
        } else {
            self.heap.push((key, val));
        }
    }

    /// The value of attribute `key`, if present (inline slots first).
    fn find(&self, key: &str) -> Option<&str> {
        for slot in self.inline[..self.len].iter().flatten() {
            if slot.0 == key {
                return Some(slot.1.as_ref());
            }
        }
        self.heap
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_ref())
    }
}

/// A parsed start tag.
pub struct Tag<'a> {
    pub name: &'a str,
    attrs: Attrs<'a>,
    pub self_closing: bool,
}

impl Tag<'_> {
    /// The value of attribute `key`, if present.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.find(key)
    }
}

/// One token.
pub enum Event<'a> {
    Open(Tag<'a>),
    Close(&'a str),
    Text(Cow<'a, str>),
}

/// A pull tokenizer over an OOXML part.
pub struct Reader<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Tokenize `s`.
    pub fn new(s: &'a str) -> Self {
        Reader { s, pos: 0 }
    }

    /// The next token, or `None` at end of input.
    pub fn read(&mut self) -> Option<Event<'a>> {
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() {
            if bytes[self.pos] != b'<' {
                return Some(self.read_text());
            }
            match bytes.get(self.pos + 1).copied().unwrap_or(b' ') {
                b'?' | b'!' => self.skip_decl(),
                b'/' => return Some(self.read_close()),
                _ => return Some(self.read_open()),
            }
        }
        None
    }

    /// Text run up to the next `<`.
    fn read_text(&mut self) -> Event<'a> {
        let start = self.pos;
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos] != b'<' {
            self.pos += 1;
        }
        Event::Text(unescape(&self.s[start..self.pos]))
    }

    /// A `</name>` close tag.
    fn read_close(&mut self) -> Event<'a> {
        self.pos += 2;
        let start = self.pos;
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos] != b'>' {
            self.pos += 1;
        }
        let name = &self.s[start..self.pos];
        self.pos += 1;
        Event::Close(name)
    }

    /// A `<name attr="v" …>` (or self-closing) open tag.
    fn read_open(&mut self) -> Event<'a> {
        self.pos += 1;
        let name = self.read_name();
        let mut attrs = Attrs::new();
        let self_closing = self.read_attrs(&mut attrs);
        Event::Open(Tag {
            name,
            attrs,
            self_closing,
        })
    }

    /// Read a tag/attribute name (stops at whitespace, `=`, `/`, `>`).
    fn read_name(&mut self) -> &'a str {
        let start = self.pos;
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() && !is_name_end(bytes[self.pos]) {
            self.pos += 1;
        }
        &self.s[start..self.pos]
    }

    /// Read attributes until the tag closes; returns whether it self-closed.
    fn read_attrs(&mut self, attrs: &mut Attrs<'a>) -> bool {
        loop {
            self.skip_ws();
            match self.s.as_bytes().get(self.pos).copied() {
                Some(b'>') => {
                    self.pos += 1;
                    return false;
                }
                Some(b'/') => {
                    self.pos += 2;
                    return true;
                }
                None => return false,
                _ => self.read_attr(attrs),
            }
        }
    }

    /// Read one `key="value"` attribute.
    fn read_attr(&mut self, attrs: &mut Attrs<'a>) {
        let key = self.read_name();
        self.skip_ws();
        if self.s.as_bytes().get(self.pos) == Some(&b'=') {
            self.pos += 1;
        }
        self.skip_ws();
        let value = self.read_quoted();
        attrs.push(key, value);
    }

    /// Read a quoted attribute value (single or double quotes).
    fn read_quoted(&mut self) -> Cow<'a, str> {
        let bytes = self.s.as_bytes();
        let quote = bytes.get(self.pos).copied().unwrap_or(b'"');
        self.pos += 1;
        let start = self.pos;
        while self.pos < bytes.len() && bytes[self.pos] != quote {
            self.pos += 1;
        }
        let value = unescape(&self.s[start..self.pos]);
        self.pos += 1;
        value
    }

    /// Skip ASCII whitespace.
    fn skip_ws(&mut self) {
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Skip a `<? … ?>` / `<! … >` declaration up to the next `>`.
    fn skip_decl(&mut self) {
        let bytes = self.s.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos] != b'>' {
            self.pos += 1;
        }
        if self.pos < bytes.len() {
            self.pos += 1;
        }
    }
}

/// Whether `b` ends a tag/attribute name.
fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'=' || b == b'/' || b == b'>'
}

/// Decode XML entities in a text/attribute run — borrowed when there is nothing
/// to decode (the common case), owned only when an `&entity;` is present.
fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        match decode_entity(after) {
            Some((ch, len)) => {
                out.push(ch);
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decode one entity at the start of `after` (which begins with `&`).
fn decode_entity(after: &str) -> Option<(char, usize)> {
    let end = after.find(';')?;
    let ch = match &after[1..end] {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        body => numeric_entity(body)?,
    };
    Some((ch, end + 1))
}

/// Decode a numeric entity body (`#123` or `#x1F`).
fn numeric_entity(body: &str) -> Option<char> {
    let num = body.strip_prefix('#')?;
    let code = match num.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => num.parse().ok()?,
    };
    char::from_u32(code)
}
