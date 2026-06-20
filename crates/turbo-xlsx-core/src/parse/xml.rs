//! A tiny, dependency-free XML tokenizer — enough to read the well-formed OOXML
//! parts an `.xlsx` contains (`<tag attr="v">text</tag>`, self-closing tags,
//! entity-escaped text). Declarations / comments (`<? … ?>`, `<! … >`) are
//! skipped. Not a general XML parser; it trusts the OOXML it is fed.

/// A parsed start tag.
pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub self_closing: bool,
}

impl Tag {
    /// The value of attribute `key`, if present.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// One token.
pub enum Event {
    Open(Tag),
    Close(String),
    Text(String),
}

/// A pull tokenizer over an OOXML part.
pub struct Reader<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Tokenize `s`.
    pub fn new(s: &'a str) -> Self {
        Reader {
            s: s.as_bytes(),
            pos: 0,
        }
    }

    /// The next token, or `None` at end of input.
    pub fn read(&mut self) -> Option<Event> {
        while self.pos < self.s.len() {
            if self.s[self.pos] != b'<' {
                return Some(self.read_text());
            }
            match self.s.get(self.pos + 1).copied().unwrap_or(b' ') {
                b'?' | b'!' => self.skip_decl(),
                b'/' => return Some(self.read_close()),
                _ => return Some(self.read_open()),
            }
        }
        None
    }

    /// Text run up to the next `<`.
    fn read_text(&mut self) -> Event {
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] != b'<' {
            self.pos += 1;
        }
        Event::Text(unescape(&self.s[start..self.pos]))
    }

    /// A `</name>` close tag.
    fn read_close(&mut self) -> Event {
        self.pos += 2;
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] != b'>' {
            self.pos += 1;
        }
        let name = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
        self.pos += 1;
        Event::Close(name)
    }

    /// A `<name attr="v" …>` (or self-closing) open tag.
    fn read_open(&mut self) -> Event {
        self.pos += 1;
        let name = self.read_name();
        let mut attrs = Vec::new();
        let self_closing = self.read_attrs(&mut attrs);
        Event::Open(Tag {
            name,
            attrs,
            self_closing,
        })
    }

    /// Read a tag/attribute name (stops at whitespace, `=`, `/`, `>`).
    fn read_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.s.len() && !is_name_end(self.s[self.pos]) {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.pos]).into_owned()
    }

    /// Read attributes until the tag closes; returns whether it self-closed.
    fn read_attrs(&mut self, attrs: &mut Vec<(String, String)>) -> bool {
        loop {
            self.skip_ws();
            match self.s.get(self.pos).copied() {
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
    fn read_attr(&mut self, attrs: &mut Vec<(String, String)>) {
        let key = self.read_name();
        self.skip_ws();
        if self.s.get(self.pos) == Some(&b'=') {
            self.pos += 1;
        }
        self.skip_ws();
        let value = self.read_quoted();
        attrs.push((key, value));
    }

    /// Read a quoted attribute value (single or double quotes).
    fn read_quoted(&mut self) -> String {
        let quote = self.s.get(self.pos).copied().unwrap_or(b'"');
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] != quote {
            self.pos += 1;
        }
        let value = unescape(&self.s[start..self.pos]);
        self.pos += 1;
        value
    }

    /// Skip ASCII whitespace.
    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Skip a `<? … ?>` / `<! … >` declaration up to the next `>`.
    fn skip_decl(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos] != b'>' {
            self.pos += 1;
        }
        if self.pos < self.s.len() {
            self.pos += 1;
        }
    }
}

/// Whether `b` ends a tag/attribute name.
fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'=' || b == b'/' || b == b'>'
}

/// Decode XML entities in a text/attribute run.
fn unescape(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if !s.contains('&') {
        return s.into_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest: &str = &s;
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
    out
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
