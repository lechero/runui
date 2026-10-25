//! Go 1.27's `encoding/json` Decoder, read a token at a time: `Token`,
//! `More` and `Decode` (into a string, or a `json.RawMessage`), as runui
//! reads package.json's scripts in their order. Go 1.27's v1 package is
//! built on `encoding/json/jsontext`, so this is jsontext's decoder state
//! machine (encoding/json/jsontext/decode.go and state.go, and
//! internal/jsonwire's consumers of literals, strings and numbers) under
//! the v1 Decoder (v2_stream.go), with v1's options: duplicate names and
//! invalid UTF-8 allowed. What it accepts, what it refuses and the words of
//! its errors are Go's: "unexpected end of JSON input", "invalid character
//! '}' after object key", "EOF".
//!
//! The input is all there from the start, as `json.NewDecoder` over the
//! file's bytes reads it: wanting more is the end of the input.

use std::fmt;

use super::quote::{decode_rune, is_print, quote_bytes, quote_rune};

/// Error is why the decoder stopped, in v1's words: the end of the input
/// (io.EOF, "EOF"), input cut short inside a token (io.ErrUnexpectedEOF,
/// "unexpected EOF"), a `*json.SyntaxError`, or a value of the wrong type
/// for what it was decoded into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Eof,
    UnexpectedEof,
    Syntax(String),
    Type(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eof => f.write_str("EOF"),
            Error::UnexpectedEof => f.write_str("unexpected EOF"),
            Error::Syntax(msg) | Error::Type(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

/// Token is a token as Decoder.Token returns it: a delimiter ([ ] { }),
/// a string, a number (its text), a bool, or null.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Delim(u8),
    Str(String),
    Number(String),
    Bool(bool),
    Null,
}

// What a syntactic error wraps: jsontext's errors, and jsonwire's
// InvalidTextError.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Inner {
    UnexpectedEof,
    Text { label: &'static str, what: Vec<u8>, place: String },
    NonStringName,
    MissingValue,
    MismatchDelim,
    MaxDepth,
}

impl Inner {
    fn words(&self) -> String {
        match self {
            Inner::UnexpectedEof => "unexpected EOF".to_string(),
            Inner::Text { label, what, place } => invalid_text_words(label, what, place),
            Inner::NonStringName => "object member name must be a string".to_string(),
            Inner::MissingValue => "missing value after object name".to_string(),
            Inner::MismatchDelim => "mismatching structural token for object or array".to_string(),
            Inner::MaxDepth => "exceeded max depth".to_string(),
        }
    }
}

// What jsontext's reads fail with: io.EOF, left as it is, or a
// *SyntacticError.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RawError {
    Eof,
    Syntactic(Inner),
}

const MAX_NESTING_DEPTH: usize = 10000;

// A stateEntry: an object or an array, and how many names and values
// (an object's count both) are in it so far.
#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    object: bool,
    count: i64,
}

impl Entry {
    fn need_object_name(self) -> bool {
        self.object && self.count % 2 == 0
    }
    fn need_object_value(self) -> bool {
        self.object && self.count % 2 == 1
    }
    fn need_implicit_comma(self, next: u8) -> bool {
        !self.need_object_value() && self.count > 0 && next != b'}' && next != b']'
    }
}

/// Decoder reads JSON a token or a value at a time: Go's `*json.Decoder`.
pub struct Decoder<'a> {
    buf: &'a [u8],
    prev_end: usize,
    // a PeekKind's result, kept for the next read: where the next token
    // starts, or why there isn't one
    peek: Option<Result<usize, RawError>>,
    stack: Vec<Entry>,
    last: Entry, // the top level is a virtual array
    // the v1 Decoder's error, kept once Decode or More meets one
    err: Option<Error>,
}

impl<'a> Decoder<'a> {
    /// new is a decoder over all of data: `json.NewDecoder(bytes.NewReader(data))`.
    pub fn new(data: &'a [u8]) -> Decoder<'a> {
        Decoder { buf: data, prev_end: 0, peek: None, stack: Vec::new(), last: Entry::default(), err: None }
    }

    /// token is the next token: Go's `Decoder.Token`. At the end of the
    /// input it's Error::Eof.
    pub fn token(&mut self) -> Result<Token, Error> {
        if let Some(e) = &self.err {
            return Err(e.clone());
        }
        match self.read_token() {
            Ok(t) => Ok(t),
            Err(RawError::Syntactic(Inner::UnexpectedEof)) => {
                // Historically, v1 would report just io.EOF if the stream is
                // a prefix of a valid JSON value. It reports an unwrapped
                // io.ErrUnexpectedEOF if truncated within a JSON token.
                let rest = &self.buf[self.prev_end..];
                if rest.iter().all(|c| matches!(c, b' ' | b'\r' | b'\n' | b'\t' | b',' | b':')) {
                    return Err(Error::Eof);
                }
                Err(Error::UnexpectedEof)
            }
            Err(e) => Err(transform(e)),
        }
    }

    /// more says there's another element in the array or object being
    /// read: Go's `Decoder.More`. An error found looking is kept, and the
    /// next read returns it.
    pub fn more(&mut self) -> bool {
        let k = self.peek_kind();
        if k == 0 {
            if self.err.is_none() {
                // PeekKind doesn't distinguish between EOF and error, so
                // read the next token to see which we get.
                let err = match self.read_token() {
                    Ok(_) => Error::Syntax("json: successful read after failed peek".to_string()),
                    Err(e) => transform(e),
                };
                self.err = Some(err);
            }
            return self.err != Some(Error::Eof);
        }
        k != b']' && k != b'}'
    }

    /// decode_raw is the next value as it's written: Go's `Decode` into a
    /// `json.RawMessage`.
    pub fn decode_raw(&mut self) -> Result<&'a [u8], Error> {
        if let Some(e) = &self.err {
            return Err(e.clone());
        }
        match self.read_value() {
            Ok(v) => Ok(v),
            Err(e) => {
                let mut err = transform(e);
                if err == Error::Syntax("unexpected end of JSON input".to_string()) {
                    // NOTE: Decode has always been inconsistent with
                    // Unmarshal with regard to the exact error value for
                    // truncated input.
                    err = Error::UnexpectedEof;
                }
                self.err = Some(err.clone());
                Err(err)
            }
        }
    }

    /// decode_string is the next value decoded into a string: Go's `Decode`
    /// into one. A null leaves the string as it was (None); a value that
    /// isn't a string is an Error::Type, the decoder going on after it.
    pub fn decode_string(&mut self) -> Result<Option<String>, Error> {
        let raw = self.decode_raw()?;
        unmarshal_string(raw)
    }

    // PeekKind: the next token's kind without reading it, 0 when there's
    // an error, kept for the read that follows.
    fn peek_kind(&mut self) -> u8 {
        if let Some(Ok(pos)) = self.peek {
            return normalize(self.buf[pos]);
        }
        let mut pos = self.prev_end;
        pos += consume_whitespace(&self.buf[pos..]);
        if pos >= self.buf.len() {
            // EOF possibly if no Tokens present after top-level value
            let err = if self.depth() == 1 { RawError::Eof } else { RawError::Syntactic(Inner::UnexpectedEof) };
            self.peek = Some(Err(err));
            return 0;
        }
        let mut delim = 0;
        let c = self.buf[pos];
        if c == b':' || c == b',' {
            delim = c;
            pos += 1;
            pos += consume_whitespace(&self.buf[pos..]);
            if pos >= self.buf.len() {
                let err = self.check_delim_before_io_error(delim, RawError::Syntactic(Inner::UnexpectedEof));
                self.peek = Some(Err(err));
                return 0;
            }
        }
        let next = normalize(self.buf[pos]);
        if self.need_delim(next) != delim {
            self.peek = Some(Err(self.check_delim(delim, next)));
            return 0;
        }
        self.peek = Some(Ok(pos));
        next
    }

    // the start of a read: where the next token is and its kind, past the
    // whitespace and the comma or colon before it, which must be the one
    // the state machine wants.
    fn next_pos(&mut self) -> Result<(usize, u8), RawError> {
        if let Some(cached) = self.peek.take() {
            let pos = cached?;
            return Ok((pos, normalize(self.buf[pos])));
        }
        let mut pos = self.prev_end;
        pos += consume_whitespace(&self.buf[pos..]);
        if pos >= self.buf.len() {
            // EOF possibly if no Tokens present after top-level value
            return Err(if self.depth() == 1 { RawError::Eof } else { RawError::Syntactic(Inner::UnexpectedEof) });
        }
        let mut delim = 0;
        let c = self.buf[pos];
        if c == b':' || c == b',' {
            delim = c;
            pos += 1;
            pos += consume_whitespace(&self.buf[pos..]);
            if pos >= self.buf.len() {
                return Err(self.check_delim_before_io_error(delim, RawError::Syntactic(Inner::UnexpectedEof)));
            }
        }
        let next = normalize(self.buf[pos]);
        if self.need_delim(next) != delim {
            return Err(self.check_delim(delim, next));
        }
        Ok((pos, next))
    }

    // ReadToken.
    fn read_token(&mut self) -> Result<Token, RawError> {
        let (mut pos, next) = self.next_pos()?;
        match next {
            b'n' | b'f' | b't' => {
                let lit: &str = match next {
                    b'n' => "null",
                    b'f' => "false",
                    _ => "true",
                };
                let (n, err) = consume_literal(&self.buf[pos..], lit);
                if let Some(err) = err {
                    return Err(self.wrap(err, pos + n));
                }
                pos += n;
                self.append_literal().map_err(|e| self.wrap(e, pos - lit.len()))?;
                self.prev_end = pos;
                Ok(match next {
                    b'n' => Token::Null,
                    b'f' => Token::Bool(false),
                    _ => Token::Bool(true),
                })
            }
            b'"' => {
                let (n, err) = consume_string(&self.buf[pos..]);
                if let Some(err) = err {
                    return Err(self.wrap(err, pos + n));
                }
                let raw = &self.buf[pos..pos + n];
                self.append_string();
                self.prev_end = pos + n;
                Ok(Token::Str(unquote(raw)))
            }
            b'0' => {
                let (n, err) = consume_number(&self.buf[pos..]);
                if let Some(err) = err {
                    return Err(self.wrap(err, pos + n));
                }
                let text = String::from_utf8_lossy(&self.buf[pos..pos + n]).into_owned();
                self.append_literal().map_err(|e| self.wrap(e, pos))?;
                self.prev_end = pos + n;
                Ok(Token::Number(text))
            }
            b'{' => {
                self.push(true).map_err(|e| self.wrap(e, pos))?;
                self.prev_end = pos + 1;
                Ok(Token::Delim(b'{'))
            }
            b'}' => {
                self.pop_object().map_err(|e| self.wrap(e, pos))?;
                self.prev_end = pos + 1;
                Ok(Token::Delim(b'}'))
            }
            b'[' => {
                self.push(false).map_err(|e| self.wrap(e, pos))?;
                self.prev_end = pos + 1;
                Ok(Token::Delim(b'['))
            }
            b']' => {
                self.pop_array().map_err(|e| self.wrap(e, pos))?;
                self.prev_end = pos + 1;
                Ok(Token::Delim(b']'))
            }
            _ => Err(self.wrap(invalid_character(&self.buf[pos..], "at start of value"), pos)),
        }
    }

    // ReadValue.
    fn read_value(&mut self) -> Result<&'a [u8], RawError> {
        let (pos, next) = self.next_pos()?;
        let depth = self.depth();
        let end = match self.consume_value(pos, depth) {
            Ok(end) => end,
            Err((at, err)) => return Err(self.wrap(err, at)),
        };
        let res = match next {
            b'n' | b't' | b'f' | b'0' => self.append_literal(),
            b'"' => {
                self.append_string();
                Ok(())
            }
            b'{' => self.push(true).and_then(|_| self.pop_object()),
            b'[' => self.push(false).and_then(|_| self.pop_array()),
            _ => Ok(()),
        };
        if let Err(e) = res {
            return Err(self.wrap(e, pos));
        }
        self.prev_end = end;
        Ok(&self.buf[pos..end])
    }

    // consumeValue: a whole value from pos, where it ends; on an error,
    // where it was found.
    fn consume_value(&self, pos: usize, depth: usize) -> Result<usize, (usize, Inner)> {
        let b = &self.buf[pos..];
        let next = normalize(b[0]);
        let (n, err) = match next {
            b'n' => consume_literal(b, "null"),
            b'f' => consume_literal(b, "false"),
            b't' => consume_literal(b, "true"),
            b'"' => consume_string(b),
            b'0' => consume_number(b),
            b'{' => return self.consume_object(pos, depth),
            b'[' => return self.consume_array(pos, depth),
            _ => {
                if (self.last.object && next == b']') || (!self.last.object && next == b'}') {
                    return Err((pos, Inner::MismatchDelim));
                }
                return Err((pos, invalid_character(b, "at start of value")));
            }
        };
        match err {
            Some(err) => Err((pos + n, err)),
            None => Ok(pos + n),
        }
    }

    // consumeObject.
    fn consume_object(&self, mut pos: usize, mut depth: usize) -> Result<usize, (usize, Inner)> {
        if depth == MAX_NESTING_DEPTH + 1 {
            return Err((pos, Inner::MaxDepth));
        }
        pos += 1;
        pos = self.whitespace(pos)?;
        if self.buf[pos] == b'}' {
            return Ok(pos + 1);
        }
        depth += 1;
        loop {
            // Handle before name.
            pos = self.whitespace(pos)?;
            let (n, err) = consume_string(&self.buf[pos..]);
            if let Some(err) = err {
                return Err((pos + n, err));
            }
            pos += n;
            // Handle after name.
            pos = self.whitespace(pos)?;
            if self.buf[pos] != b':' {
                return Err((pos, invalid_character(&self.buf[pos..], "after object name (expecting ':')")));
            }
            pos += 1;
            // Handle before value.
            pos = self.whitespace(pos)?;
            pos = self.consume_value(pos, depth)?;
            // Handle after value.
            pos = self.whitespace(pos)?;
            match self.buf[pos] {
                b',' => pos += 1,
                b'}' => return Ok(pos + 1),
                _ => {
                    return Err((
                        pos,
                        invalid_character(&self.buf[pos..], "after object value (expecting ',' or '}')"),
                    ));
                }
            }
        }
    }

    // consumeArray.
    fn consume_array(&self, mut pos: usize, mut depth: usize) -> Result<usize, (usize, Inner)> {
        if depth == MAX_NESTING_DEPTH + 1 {
            return Err((pos, Inner::MaxDepth));
        }
        pos += 1;
        pos = self.whitespace(pos)?;
        if self.buf[pos] == b']' {
            return Ok(pos + 1);
        }
        depth += 1;
        loop {
            // Handle before value.
            pos = self.whitespace(pos)?;
            pos = self.consume_value(pos, depth)?;
            // Handle after value.
            pos = self.whitespace(pos)?;
            match self.buf[pos] {
                b',' => pos += 1,
                b']' => return Ok(pos + 1),
                _ => {
                    return Err((
                        pos,
                        invalid_character(&self.buf[pos..], "after array element (expecting ',' or ']')"),
                    ));
                }
            }
        }
    }

    // consumeWhitespace: past the whitespace from pos, an error at the end
    // of the input.
    fn whitespace(&self, pos: usize) -> Result<usize, (usize, Inner)> {
        let pos = pos + consume_whitespace(&self.buf[pos..]);
        if pos >= self.buf.len() {
            return Err((pos, Inner::UnexpectedEof));
        }
        Ok(pos)
    }

    // checkDelim: why delim, or its absence, is wrong before a token of
    // kind next.
    fn check_delim(&self, delim: u8, next: u8) -> RawError {
        let place = match self.need_delim(next) {
            d if d == delim => return RawError::Syntactic(Inner::UnexpectedEof), // never: it's right
            b':' => "after object name (expecting ':')",
            b',' if self.last.object => "after object value (expecting ',' or '}')",
            b',' => "after array element (expecting ',' or ']')",
            _ => "at start of value",
        };
        let pos = self.prev_end + consume_whitespace(&self.buf[self.prev_end..]);
        RawError::Syntactic(invalid_character(&self.buf[pos..], place))
    }

    // checkDelimBeforeIOError: whether the delim is even valid before
    // the end of the input that follows it, taking a string as next, which
    // is always valid there.
    fn check_delim_before_io_error(&self, delim: u8, err: RawError) -> RawError {
        if self.need_delim(b'"') != delim {
            return self.check_delim(delim, b'"');
        }
        err
    }

    // wrapSyntacticError: a mismatched delimiter said as the character
    // found, where it was found.
    fn wrap(&self, err: Inner, pos: usize) -> RawError {
        if err != Inner::MismatchDelim {
            return RawError::Syntactic(err);
        }
        let mut place = "at start of value";
        if !self.stack.is_empty() && self.last.count > 0 {
            place = if self.last.object {
                "after object value (expecting ',' or '}')"
            } else {
                "after array element (expecting ',' or ']')"
            };
        }
        RawError::Syntactic(invalid_character(&self.buf[pos.min(self.buf.len())..], place))
    }

    fn depth(&self) -> usize {
        self.stack.len() + 1
    }

    fn need_delim(&self, next: u8) -> u8 {
        if self.last.need_object_value() {
            b':'
        } else if self.last.need_implicit_comma(next) && !self.stack.is_empty() {
            // comma not needed for top-level values
            b','
        } else {
            0
        }
    }

    fn append_literal(&mut self) -> Result<(), Inner> {
        if self.last.need_object_name() {
            return Err(Inner::NonStringName);
        }
        self.last.count += 1;
        Ok(())
    }

    fn append_string(&mut self) {
        self.last.count += 1;
    }

    fn push(&mut self, object: bool) -> Result<(), Inner> {
        if self.last.need_object_name() {
            return Err(Inner::NonStringName);
        }
        if self.stack.len() == MAX_NESTING_DEPTH {
            return Err(Inner::MaxDepth);
        }
        self.last.count += 1;
        self.stack.push(self.last);
        self.last = Entry { object, count: 0 };
        Ok(())
    }

    fn pop_object(&mut self) -> Result<(), Inner> {
        if !self.last.object {
            return Err(Inner::MismatchDelim);
        }
        if self.last.need_object_value() {
            return Err(Inner::MissingValue);
        }
        self.last = self.stack.pop().unwrap_or_default();
        Ok(())
    }

    fn pop_array(&mut self) -> Result<(), Inner> {
        // forbid popping top-level virtual JSON array
        if self.last.object || self.stack.is_empty() {
            return Err(Inner::MismatchDelim);
        }
        self.last = self.stack.pop().unwrap_or_default();
        Ok(())
    }
}

/// check_valid says whether data is one JSON value, whitespace around it
/// aside, in the words of Go's `json.Unmarshal` when it isn't:
/// "unexpected end of JSON input", "invalid character 'x' after
/// top-level value".
pub fn check_valid(data: &[u8]) -> Result<(), Error> {
    let mut d = Decoder::new(data);
    match d.read_value() {
        Err(RawError::Eof) => return Err(Error::Syntax("unexpected end of JSON input".to_string())),
        Err(e) => return Err(transform(e)),
        Ok(_) => {}
    }
    let pos = d.prev_end + consume_whitespace(&data[d.prev_end..]);
    if pos < data.len() {
        return Err(transform(RawError::Syntactic(invalid_character(&data[pos..], "after top-level value"))));
    }
    Ok(())
}

/// unmarshal_string is a raw value decoded into a string: Go's
/// `json.Unmarshal(raw, &s)`. A null leaves the string as it was (None);
/// a value that isn't a string is an Error::Type.
pub fn unmarshal_string(raw: &[u8]) -> Result<Option<String>, Error> {
    match raw.first() {
        Some(b'"') => Ok(Some(unquote(raw))),
        Some(b'n') => Ok(None),
        first => {
            let kind = match first {
                Some(b'{') => "object",
                Some(b'[') => "array",
                Some(b't' | b'f') => "bool",
                _ => "number",
            };
            Err(Error::Type(format!("json: cannot unmarshal {kind} into Go value of type string")))
        }
    }
}

// transformSyntacticError: a jsontext error in v1's words.
fn transform(err: RawError) -> Error {
    match err {
        RawError::Eof => Error::Eof,
        RawError::Syntactic(Inner::UnexpectedEof) => Error::Syntax("unexpected end of JSON input".to_string()),
        RawError::Syntactic(inner) => {
            let mut msg = inner.words();
            if let Some(i) = msg.find(" (expecting")
                && !msg.contains(" in literal")
            {
                msg.truncate(i);
            }
            // the v2 error's words, as v1 said them
            for (from, to) in [
                ("object name", "object key"),
                ("at start of value", "looking for beginning of value"),
                ("at start of string", "looking for beginning of object key string"),
                ("after object value", "after object key:value pair"),
                ("in number", "in numeric literal"),
            ] {
                msg = msg.replace(from, to);
            }
            Error::Syntax(msg)
        }
    }
}

// normKind: a token's kind by its first byte, 0 for none.
fn normalize(c: u8) -> u8 {
    match c {
        b'n' | b'f' | b't' | b'"' | b'{' | b'}' | b'[' | b']' => c,
        b'-' | b'0'..=b'9' => b'0',
        _ => 0,
    }
}

fn consume_whitespace(b: &[u8]) -> usize {
    b.iter().take_while(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n')).count()
}

fn invalid_character(prefix: &[u8], place: &str) -> Inner {
    let (_, n) = decode_rune(prefix);
    Inner::Text { label: "character", what: prefix[..n].to_vec(), place: place.to_string() }
}

fn invalid_escape_sequence(what: &[u8]) -> Inner {
    let label = if what.len() > 6 { "surrogate pair" } else { "escape sequence" };
    Inner::Text { label, what: what.to_vec(), place: "in string".to_string() }
}

// InvalidTextError.Error: the text found, quoted as Go quotes it.
fn invalid_text_words(label: &str, what: &[u8], place: &str) -> String {
    let runes = rune_count(what);
    let need_escape = {
        let mut i = 0;
        let mut need = false;
        while i < what.len() {
            let (r, n) = decode_rune(&what[i..]);
            match r {
                Some(r) if r != '`' && r != '\u{FFFD}' && !r.is_whitespace() && is_print(r) => {}
                _ => need = true,
            }
            i += n.max(1);
        }
        need
    };
    let shown = if runes == 1 {
        jsonwire_quote_rune(what)
    } else if need_escape {
        quote_bytes(what)
    } else {
        format!("`{}`", String::from_utf8_lossy(what))
    };
    let s = format!("invalid {label} {shown} {place}");
    s.strip_suffix(' ').map(str::to_string).unwrap_or(s)
}

// jsonwire.QuoteRune: the first character in single quotes, or a byte
// that doesn't start one as '\xff'.
fn jsonwire_quote_rune(b: &[u8]) -> String {
    match decode_rune(b) {
        (Some(r), _) => quote_rune(r),
        (None, _) => format!("'\\x{:x}'", b.first().copied().unwrap_or(0)),
    }
}

fn rune_count(b: &[u8]) -> usize {
    let (mut i, mut n) = (0, 0);
    while i < b.len() {
        let (_, w) = decode_rune(&b[i..]);
        i += w.max(1);
        n += 1;
    }
    n
}

// ConsumeLiteral: how much of lit b starts with, and why it doesn't go on.
fn consume_literal(b: &[u8], lit: &str) -> (usize, Option<Inner>) {
    let lit = lit.as_bytes();
    for i in 0..b.len().min(lit.len()) {
        if b[i] != lit[i] {
            let place =
                format!("in literal {} (expecting {})", String::from_utf8_lossy(lit), quote_rune(char::from(lit[i])));
            return (i, Some(invalid_character(&b[i..], &place)));
        }
    }
    if b.len() < lit.len() {
        return (b.len(), Some(Inner::UnexpectedEof));
    }
    (lit.len(), None)
}

// ConsumeString, without checking the UTF-8: how long the string at the
// start of b is, or where it goes wrong.
fn consume_string(b: &[u8]) -> (usize, Option<Inner>) {
    let mut n = 0;
    match b.first() {
        None => return (0, Some(Inner::UnexpectedEof)),
        Some(b'"') => n += 1,
        Some(_) => return (0, Some(invalid_character(b, "at start of string (expecting '\"')"))),
    }
    loop {
        while n < b.len() && b[n] < 0x80 && b[n] >= b' ' && b[n] != b'\\' && b[n] != b'"' {
            n += 1;
        }
        if n >= b.len() {
            return (n, Some(Inner::UnexpectedEof));
        }
        if b[n] == b'"' {
            return (n + 1, None);
        }
        let (r, rn) = decode_rune(&b[n..]);
        match r {
            // Handle UTF-8 encoded byte sequence.
            Some(_) if rn > 1 => n += rn,
            // Handle escape sequence.
            Some('\\') => {
                if b.len() < n + 2 {
                    return (n, Some(Inner::UnexpectedEof));
                }
                match b[n + 1] {
                    b'/' | b'"' | b'\\' | b'b' | b'f' | b'n' | b'r' | b't' => n += 2,
                    b'u' => {
                        if b.len() < n + 6 {
                            if has_escaped_utf16_prefix(&b[n..], false) {
                                return (n, Some(Inner::UnexpectedEof));
                            }
                            return (n, Some(invalid_escape_sequence(&b[n..])));
                        }
                        if parse_hex_u16(&b[n + 2..n + 6]).is_none() {
                            return (n, Some(invalid_escape_sequence(&b[n..n + 6])));
                        }
                        n += 6;
                    }
                    _ => return (n, Some(invalid_escape_sequence(&b[n..n + 2]))),
                }
            }
            // Handle invalid UTF-8.
            None => {
                if !full_rune(&b[n..]) {
                    return (n, Some(Inner::UnexpectedEof));
                }
                n += 1;
            }
            // Handle invalid control characters.
            Some(_) => {
                return (n, Some(invalid_character(&b[n..], "in string (expecting non-control character)")));
            }
        }
    }
}

// ConsumeNumber: how long the number at the start of b is, or where it
// goes wrong; a number cut short by the end of the input is an
// unexpected EOF, one that could end there ends there.
fn consume_number(b: &[u8]) -> (usize, Option<Inner>) {
    let mut n = 0;
    let digit = |c: u8| c.is_ascii_digit();
    // Consume required integer component (with optional minus sign).
    if b.first() == Some(&b'-') {
        n += 1;
    }
    if n >= b.len() {
        return (0, Some(Inner::UnexpectedEof));
    }
    match b[n] {
        b'0' => n += 1,
        b'1'..=b'9' => {
            n += 1;
            while n < b.len() && digit(b[n]) {
                n += 1;
            }
        }
        _ => return (n, Some(invalid_character(&b[n..], "in number (expecting digit)"))),
    }
    // Consume optional fractional component.
    if n < b.len() && b[n] == b'.' {
        n += 1;
        if n >= b.len() {
            return (0, Some(Inner::UnexpectedEof));
        }
        if !digit(b[n]) {
            return (n, Some(invalid_character(&b[n..], "in number (expecting digit)")));
        }
        while n < b.len() && digit(b[n]) {
            n += 1;
        }
    }
    // Consume optional exponent component.
    if n < b.len() && (b[n] == b'e' || b[n] == b'E') {
        n += 1;
        if n < b.len() && (b[n] == b'-' || b[n] == b'+') {
            n += 1;
        }
        if n >= b.len() {
            return (0, Some(Inner::UnexpectedEof));
        }
        if !digit(b[n]) {
            return (n, Some(invalid_character(&b[n..], "in number (expecting digit)")));
        }
        while n < b.len() && digit(b[n]) {
            n += 1;
        }
    }
    (n, None)
}

// hasEscapedUTF16Prefix: whether b may be the start of a \uFFFF escape
// cut short.
fn has_escaped_utf16_prefix(b: &[u8], lower_surrogate_half: bool) -> bool {
    for (i, &c) in b.iter().enumerate() {
        let hex = c.is_ascii_hexdigit();
        match i {
            0 if c != b'\\' => return false,
            1 if c != b'u' => return false,
            2 if lower_surrogate_half && c != b'd' && c != b'D' => return false,
            3 if lower_surrogate_half && !(b'c'..=b'f').contains(&c) && !(b'C'..=b'F').contains(&c) => return false,
            2..=5 if !hex => return false,
            _ => {}
        }
    }
    true
}

fn parse_hex_u16(b: &[u8]) -> Option<u16> {
    if b.len() != 4 {
        return None;
    }
    let mut v: u16 = 0;
    for &c in b {
        let d = (c as char).to_digit(16)? as u16;
        v = v * 16 + d;
    }
    Some(v)
}

// utf8.FullRune: whether b begins with a whole encoding of a character,
// or an encoding that can't be one, rather than one cut short.
fn full_rune(b: &[u8]) -> bool {
    let Some(&c) = b.first() else { return false };
    let need = match c {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return true, // an invalid first byte is a whole error
    };
    if b.len() >= need {
        return true;
    }
    // a continuation byte that can't follow is an error now, not a cut
    let (lo, hi) = match c {
        0xe0 => (0xa0, 0xbf),
        0xed => (0x80, 0x9f),
        0xf0 => (0x90, 0xbf),
        0xf4 => (0x80, 0x8f),
        _ => (0x80, 0xbf),
    };
    if b.len() > 1 && !(lo..=hi).contains(&b[1]) {
        return true;
    }
    if b.len() > 2 && !(0x80..=0xbf).contains(&b[2]) {
        return true;
    }
    false
}

// AppendUnquote, its errors left out (UnquoteMayCopy): a JSON string's
// text, bytes that aren't UTF-8 and escapes that don't make a character
// each U+FFFD.
fn unquote(src: &[u8]) -> String {
    let mut dst: Vec<u8> = Vec::with_capacity(src.len());
    if src.first() != Some(&b'"') {
        return String::new();
    }
    let (mut i, mut n) = (1, 1);
    let push_rune = |dst: &mut Vec<u8>, r: char| {
        let mut b = [0u8; 4];
        dst.extend_from_slice(r.encode_utf8(&mut b).as_bytes());
    };
    while n < src.len() {
        while n < src.len() && src[n] < 0x80 && src[n] >= b' ' && src[n] != b'\\' && src[n] != b'"' {
            n += 1;
        }
        if n >= src.len() {
            dst.extend_from_slice(&src[i..n]);
            break;
        }
        if src[n] == b'"' {
            dst.extend_from_slice(&src[i..n]);
            break;
        }
        let (r, rn) = decode_rune(&src[n..]);
        match r {
            Some(_) if rn > 1 => n += rn,
            Some('\\') => {
                dst.extend_from_slice(&src[i..n]);
                if src.len() < n + 2 {
                    break;
                }
                match src[n + 1] {
                    c @ (b'"' | b'\\' | b'/') => {
                        dst.push(c);
                        n += 2;
                    }
                    b'b' => {
                        dst.push(0x08);
                        n += 2;
                    }
                    b'f' => {
                        dst.push(0x0c);
                        n += 2;
                    }
                    b'n' => {
                        dst.push(b'\n');
                        n += 2;
                    }
                    b'r' => {
                        dst.push(b'\r');
                        n += 2;
                    }
                    b't' => {
                        dst.push(b'\t');
                        n += 2;
                    }
                    b'u' => {
                        if src.len() < n + 6 {
                            break;
                        }
                        let Some(v1) = parse_hex_u16(&src[n + 2..n + 6]) else { break };
                        n += 6;
                        let mut r = char::from_u32(u32::from(v1));
                        // Check whether this is a surrogate half.
                        if (0xd800..0xe000).contains(&v1) {
                            r = None; // assume failure unless the following succeeds
                            if src.len() >= n + 6
                                && src[n] == b'\\'
                                && src[n + 1] == b'u'
                                && let Some(v2) = parse_hex_u16(&src[n + 2..n + 6])
                                && (0xd800..0xdc00).contains(&v1)
                                && (0xdc00..0xe000).contains(&v2)
                            {
                                let c = 0x10000 + ((u32::from(v1) - 0xd800) << 10) + (u32::from(v2) - 0xdc00);
                                r = char::from_u32(c);
                                n += 6;
                            }
                        }
                        push_rune(&mut dst, r.unwrap_or('\u{FFFD}'));
                    }
                    _ => break,
                }
                i = n;
            }
            None => {
                dst.extend_from_slice(&src[i..n]);
                if !full_rune(&src[n..]) {
                    break;
                }
                // NOTE: An unescaped string may be longer than the escaped
                // string because invalid UTF-8 bytes are being replaced.
                push_rune(&mut dst, '\u{FFFD}');
                n += 1;
                i = n;
            }
            Some(_) => {
                dst.extend_from_slice(&src[i..n]);
                break;
            }
        }
    }
    // the bytes kept are whole characters but for a cut-short end, as Go's
    super::quote::go_lossy(&dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    // readScripts' way through package.json: the object's fields, its
    // scripts in order, each a string or skipped.
    fn scripts(data: &str) -> Result<Vec<(String, String)>, String> {
        let mut dec = Decoder::new(data.as_bytes());
        let expect = |dec: &mut Decoder| -> Result<(), String> {
            match dec.token() {
                Err(e) => Err(e.to_string()),
                Ok(Token::Delim(b'{')) => Ok(()),
                Ok(_) => Err("not a JSON object".to_string()),
            }
        };
        expect(&mut dec)?;
        let mut out = Vec::new();
        while dec.more() {
            let field = dec.decode_string().map_err(|e| e.to_string())?.unwrap_or_default();
            if field == "scripts" {
                expect(&mut dec).map_err(|e| format!("scripts: {e}"))?;
                out.clear();
                while dec.more() {
                    let name = dec.decode_string().map_err(|e| format!("scripts: {e}"))?.unwrap_or_default();
                    let raw = dec.decode_raw().map_err(|e| format!("scripts: {e}"))?;
                    if let Ok(line) = unmarshal_string(raw) {
                        out.push((name, line.unwrap_or_default()));
                    }
                }
                dec.token().map_err(|e| format!("scripts: {e}"))?;
            } else {
                dec.decode_raw().map_err(|e| e.to_string())?;
            }
        }
        Ok(out)
    }

    // what Go 1.27's ReadPackageJSON said of each, less its "package.json: "
    #[test]
    fn package_json_reads_as_go_reads_it() {
        for (body, want) in [
            ("{", Err("unexpected end of JSON input")),
            (r#"["dev"]"#, Err("not a JSON object")),
            (r#"{"scripts": {"dev": "vite""#, Err("scripts: unexpected end of JSON input")),
            (r#"{"scripts": ["dev"]}"#, Err("scripts: not a JSON object")),
            (r#"{"name": "x"}"#, Ok("")),
            ("", Err("EOF")),
            ("  ", Err("EOF")),
            ("x", Err("invalid character 'x' looking for beginning of value")),
            (r#"{"a" 1}"#, Err("invalid character '1' after object key")),
            (r#"{"a": 1 "b": 2}"#, Err("invalid character '\"' after object key:value pair")),
            (r#"{"scripts": {"a": "b",}}"#, Err("scripts: invalid character ',' looking for beginning of value")),
            (r#"{"scripts": {"a": "b"}} trailing"#, Ok("a=b")),
            (r#"{"scripts": {"a": "b"}"#, Err("unexpected end of JSON input")),
            (r#"{"scripts": {"a": tru}}"#, Err("scripts: invalid character '}' in literal true (expecting 'e')")),
            (r#"{"scripts": {"a": "b\x01"}}"#, Err("scripts: invalid escape sequence `\\x` in string")),
            (r#"{"x": [1, 2,]}"#, Err("invalid character ']' after object key:value pair")),
            (r#"{"x": {"y": }}"#, Err("invalid character '}' looking for beginning of value")),
            (r#"{"scripts": {"a": "b", "c": 1, "d": null, "e": "f"}}"#, Ok("a=b d= e=f")),
            (r#"{"packageManager": 5, "scripts": {"a": "b"}}"#, Ok("a=b")),
            (r#"{"scripts": {"a": "b"}, "scripts": {"c": "d"}}"#, Ok("c=d")),
            ("{1: 2}", Err("object member name must be a string")),
            (r#"{"x": "\u12"}"#, Err("invalid escape sequence `\\u12\"}` in string")),
            (r#"{"x": 01}"#, Err("invalid character '1' after object key:value pair")),
            (r#"{"x": -}"#, Err("invalid character '}' in numeric literal")),
            (r#"{"x": 1.}"#, Err("invalid character '}' in numeric literal")),
            (r#"{"x": 1e}"#, Err("invalid character '}' in numeric literal")),
            (r#"{"x": nul}"#, Err("invalid character '}' in literal null (expecting 'l')")),
            (r#"{"x": "a"#, Err("unexpected EOF")),
            (r#"{"x": ["#, Err("unexpected EOF")),
            (r#"{"x": [1"#, Err("unexpected EOF")),
            (r#"{"x""#, Err("unexpected EOF")),
            (r#"{"x":"#, Err("unexpected EOF")),
            (r#"{"scripts": {"a""#, Err("scripts: unexpected EOF")),
            (r#"{"scripts": {"a":"#, Err("scripts: unexpected EOF")),
            (r#"{"scripts": {"a": "b","#, Err("scripts: unexpected end of JSON input")),
            (r#"{"scripts": {"a": "b" "c"}}"#, Err("scripts: invalid character '\"' after object key:value pair")),
            ("{\"x\": \"\u{00ff}\"}", Ok("")),
            (r#"{"scripts": {"a\u0041": "b\u00e9"}}"#, Ok("aA=bé")),
        ] {
            let got = scripts(body).map(|s| s.iter().map(|(n, l)| format!("{n}={l}")).collect::<Vec<_>>().join(" "));
            let want: Result<&str, &str> = want;
            assert_eq!(got.as_deref().map_err(String::as_str), want, "{body:?}");
        }
    }

    #[test]
    fn unmarshal_checks_as_gos() {
        assert_eq!(check_valid(b" {\"a\": [1]} "), Ok(()));
        assert_eq!(check_valid(b""), Err(Error::Syntax("unexpected end of JSON input".to_string())));
        assert_eq!(check_valid(b"{\"a\""), Err(Error::Syntax("unexpected end of JSON input".to_string())));
        assert_eq!(
            check_valid(b"{not json"),
            Err(Error::Syntax("invalid character 'n' looking for beginning of object key string".to_string()))
        );
        assert_eq!(check_valid(b"{} x"), Err(Error::Syntax("invalid character 'x' after top-level value".to_string())));
    }

    #[test]
    fn strings_unquote_as_gos() {
        assert_eq!(unquote(br#""a\nb""#), "a\nb");
        assert_eq!(unquote(b"\"\xff\xfe\""), "\u{FFFD}\u{FFFD}");
        assert_eq!(unquote(br#""\ud83d\ude00""#), "😀");
        assert_eq!(unquote(br#""\ud800x""#), "\u{FFFD}x");
        assert_eq!(unquote(br#""\ud800\u0041""#), "\u{FFFD}A");
        assert_eq!(unmarshal_string(b"null"), Ok(None));
        assert!(unmarshal_string(b"42").is_err());
    }
}
