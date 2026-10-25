//! Go's encoding/xml decoder, its tokenizer, so a pom.xml reads as the Go
//! build reads it: what it takes and what it refuses (Strict, or Go's
//! forgiving mode), its namespaces, entities and line ends, and its errors
//! word for word (`XML syntax error on line 5: element <modules> closed by
//! </project>`). EscapeText too. Taken from aemui's aem-vault.
//!
//! Go decodes into structs by reflection; here the callers walk the tokens
//! (`token`, `skip`) the way `Decoder.Decode` does for theirs. A
//! CharsetReader isn't there: a document declaring another encoding than
//! UTF-8 is refused, as Go refuses it without one.

use std::collections::HashMap;
use std::fmt;

/// SyntaxError is a syntax error in the XML, and on which line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub msg: String,
    pub line: i64,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "XML syntax error on line {}: {}", self.line, self.msg)
    }
}

impl std::error::Error for SyntaxError {}

/// Error is why decoding stopped: the end of the input (Go's io.EOF, "EOF"),
/// a syntax error, or another of Go's errors in its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Eof,
    Syntax(SyntaxError),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eof => f.write_str("EOF"),
            Error::Syntax(e) => e.fmt(f),
            Error::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

/// Name is an XML name: its local part, and its name space, which tokens
/// give as the URL the prefix stands for when it's known.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Name {
    pub space: String,
    pub local: String,
}

/// Attr is an attribute of an element: name="value".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attr {
    pub name: Name,
    pub value: String,
}

/// StartElement is an element's start: `<name attr="value">`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartElement {
    pub name: Name,
    pub attr: Vec<Attr>,
}

/// EndElement is an element's end: `</name>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndElement {
    pub name: Name,
}

/// ProcInst is a processing instruction: `<?target inst?>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcInst {
    pub target: String,
    pub inst: Vec<u8>,
}

/// Token is a piece of XML: an element's start or end, character data
/// (text and CDATA, entities replaced: always UTF-8), a comment, a
/// processing instruction, or a directive (`<!DOCTYPE ...>`, its `<!`
/// and `>` left off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    StartElement(StartElement),
    EndElement(EndElement),
    CharData(String),
    Comment(Vec<u8>),
    ProcInst(ProcInst),
    Directive(Vec<u8>),
}

const XML_URL: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_PREFIX: &str = "xmlns";
const XML_PREFIX: &str = "xml";

/// Stack holds the open elements and the name space translations to undo
/// when each ends; the translations to undo when an element ends are below
/// it.
#[derive(Debug, Clone)]
struct Stack {
    start: bool, // an element's start, or else a name space translation
    name: Name,
    ok: bool,
}

/// Decoder reads XML from bytes, a token at a time.
pub struct Decoder<'a> {
    /// strict, true unless set otherwise, holds to the XML specification.
    /// When false, common mistakes go: a missing end tag is made up, an
    /// unknown or broken entity is left as it is, an attribute's value may
    /// go unquoted, or missing, its name standing for it.
    pub strict: bool,
    /// auto_close are the elements closed as soon as they're opened, when
    /// strict is false, whether an end tag follows or not.
    pub auto_close: Vec<String>,
    /// entity maps entity names besides lt, gt, amp, apos and quot to what
    /// they stand for.
    pub entity: HashMap<String, String>,
    /// default_space is the name space of names without one, as if the
    /// whole document were in an element with xmlns="default_space".
    pub default_space: String,

    data: &'a [u8],
    pos: usize,
    buf: Vec<u8>,
    stk: Vec<Stack>,
    need_close: bool,
    to_close: Name,
    next_token: Option<Token>,
    next_byte: Option<u8>,
    ns: HashMap<String, String>,
    err: Option<Error>,
    line: i64,
    linestart: i64,
    offset: i64,
}

impl<'a> Decoder<'a> {
    /// new is a decoder of data, strict.
    pub fn new(data: &'a [u8]) -> Decoder<'a> {
        Decoder {
            strict: true,
            auto_close: Vec::new(),
            entity: HashMap::new(),
            default_space: String::new(),
            data,
            pos: 0,
            buf: Vec::new(),
            stk: Vec::new(),
            need_close: false,
            to_close: Name::default(),
            next_token: None,
            next_byte: None,
            ns: HashMap::new(),
            err: None,
            line: 1,
            linestart: 0,
            offset: 0,
        }
    }

    /// token is the next token, Error::Eof at the end. A self-closing
    /// element is a start and then an end; starts and ends are matched, an
    /// end without its start or the input ending inside an element being
    /// an error. Names are translated to their name spaces' URLs; an
    /// unknown prefix stays as the space.
    pub fn token(&mut self) -> Result<Token, Error> {
        let mut t = match self.next_token.take() {
            Some(t) => t,
            None => match self.raw_token_inner() {
                Ok(t) => t,
                Err(err) => {
                    if err == Error::Eof && !self.stk.is_empty() {
                        return Err(self.syntax_error("unexpected EOF"));
                    }
                    return Err(err);
                }
            },
        };
        if !self.strict
            && let Some(t1) = self.auto_close_token(&t)
        {
            self.next_token = Some(t);
            t = t1;
        }
        match t {
            Token::StartElement(mut t1) => {
                // in XML name spaces, the translations listed in the
                // attributes apply to the element name and to the other
                // attribute names, so they come first
                for a in &t1.attr {
                    if a.name.space == XMLNS_PREFIX {
                        let v = self.ns.get(&a.name.local).cloned();
                        self.push_ns(&a.name.local, v);
                        self.ns.insert(a.name.local.clone(), a.value.clone());
                    }
                    if a.name.space.is_empty() && a.name.local == XMLNS_PREFIX {
                        // default space for untagged names
                        let v = self.ns.get("").cloned();
                        self.push_ns("", v);
                        self.ns.insert(String::new(), a.value.clone());
                    }
                }
                self.stk.push(Stack { start: true, name: t1.name.clone(), ok: false });
                self.translate(&mut t1.name, true);
                for i in 0..t1.attr.len() {
                    let mut name = std::mem::take(&mut t1.attr[i].name);
                    self.translate(&mut name, false);
                    t1.attr[i].name = name;
                }
                Ok(Token::StartElement(t1))
            }
            Token::EndElement(mut t1) => {
                if !self.pop_element(&mut t1) {
                    return Err(self.err.clone().expect("an error set"));
                }
                Ok(Token::EndElement(t1))
            }
            t => Ok(t),
        }
    }

    /// raw_token is like token without checking that starts and ends match
    /// and without translating name space prefixes.
    pub fn raw_token(&mut self) -> Result<Token, Error> {
        self.raw_token_inner()
    }

    /// skip reads tokens until the end of the element whose start was read
    /// last, the elements in it too.
    pub fn skip(&mut self) -> Result<(), Error> {
        let mut depth: i64 = 0;
        loop {
            match self.token()? {
                Token::StartElement(_) => depth += 1,
                Token::EndElement(_) => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }

    /// input_offset is how far into the input the decoder is: the end of
    /// the token read last and the start of the next.
    pub fn input_offset(&self) -> i64 {
        self.offset
    }

    /// input_pos is the line the decoder is on and the column, from 1, of
    /// the end of the token read last.
    pub fn input_pos(&self) -> (i64, i64) {
        (self.line, self.offset - self.linestart + 1)
    }

    /// translate applies the name space translation to n. The default
    /// name space (an empty space) applies only to element names.
    fn translate(&self, n: &mut Name, is_element_name: bool) {
        if n.space == XMLNS_PREFIX
            || (n.space.is_empty() && !is_element_name)
            || (n.space.is_empty() && n.local == XMLNS_PREFIX)
        {
            return;
        }
        if n.space == XML_PREFIX {
            n.space = XML_URL.to_string();
        }
        if let Some(v) = self.ns.get(&n.space) {
            n.space = v.clone();
        } else if n.space.is_empty() {
            n.space = self.default_space.clone();
        }
    }

    /// push_ns records that ns[local] changes; it was url, or wasn't there.
    fn push_ns(&mut self, local: &str, url: Option<String>) {
        let ok = url.is_some();
        self.stk.push(Stack {
            start: false,
            name: Name { space: url.unwrap_or_default(), local: local.to_string() },
            ok,
        });
    }

    fn syntax_error(&self, msg: &str) -> Error {
        Error::Syntax(SyntaxError { msg: msg.to_string(), line: self.line })
    }

    fn set_syntax_error(&mut self, msg: &str) -> Error {
        let e = self.syntax_error(msg);
        self.err = Some(e.clone());
        e
    }

    /// pop_element records that an element with t's name ends: the start
    /// on top of the stack must match, and the translations under it are
    /// undone.
    fn pop_element(&mut self, t: &mut EndElement) -> bool {
        let s = self.stk.pop();
        let name = t.name.clone();
        match s {
            Some(s) if s.start => {
                if s.name.local != name.local {
                    if !self.strict {
                        self.need_close = true;
                        self.to_close = t.name.clone();
                        t.name = s.name;
                        return true;
                    }
                    self.set_syntax_error(&format!("element <{}> closed by </{}>", s.name.local, name.local));
                    return false;
                }
                if s.name.space != name.space {
                    let ns = if name.space.is_empty() { "\"\"".to_string() } else { name.space.clone() };
                    self.set_syntax_error(&format!(
                        "element <{}> in space {} closed by </{}> in space {}",
                        s.name.local, s.name.space, name.local, ns
                    ));
                    return false;
                }
            }
            _ => {
                self.set_syntax_error(&format!("unexpected end element </{}>", name.local));
                return false;
            }
        }

        self.translate(&mut t.name, true);

        // undo the translations that came with the element just closed
        while let Some(s) = self.stk.last() {
            if s.start {
                break;
            }
            let s = self.stk.pop().expect("there");
            if s.ok {
                self.ns.insert(s.name.local, s.name.space);
            } else {
                self.ns.remove(&s.name.local);
            }
        }
        true
    }

    /// auto_close_token is the end tag made up when the element on top is
    /// one that closes itself and t doesn't close it.
    fn auto_close_token(&self, t: &Token) -> Option<Token> {
        let top = self.stk.last().filter(|s| s.start)?;
        for s in &self.auto_close {
            if s.eq_ignore_ascii_case(&top.name.local) || equal_fold(s, &top.name.local) {
                // closed here unless t closes it
                return match t {
                    Token::EndElement(et) if equal_fold(&et.name.local, &top.name.local) => None,
                    _ => Some(Token::EndElement(EndElement { name: top.name.clone() })),
                };
            }
        }
        None
    }

    fn raw_token_inner(&mut self) -> Result<Token, Error> {
        if let Some(e) = &self.err {
            return Err(e.clone());
        }
        if self.need_close {
            // the element read last closed itself, and only its start was
            // returned: its end now
            self.need_close = false;
            return Ok(Token::EndElement(EndElement { name: std::mem::take(&mut self.to_close) }));
        }

        let Some(b) = self.getc() else {
            return Err(self.err.clone().expect("an error set"));
        };

        if b != b'<' {
            // text
            self.ungetc(b);
            let Some(data) = self.text(-1, false) else {
                return Err(self.err.clone().expect("an error set"));
            };
            return Ok(Token::CharData(String::from_utf8(data).expect("checked")));
        }

        let Some(mut b) = self.mustgetc() else {
            return Err(self.err.clone().expect("an error set"));
        };
        match b {
            b'/' => {
                // </: end element
                let Some(name) = self.nsname() else {
                    return Err(self.err_or("expected element name after </"));
                };
                self.space();
                let Some(b) = self.mustgetc() else {
                    return Err(self.err.clone().expect("an error set"));
                };
                if b != b'>' {
                    return Err(self.set_syntax_error(&format!("invalid characters between </{} and >", name.local)));
                }
                return Ok(Token::EndElement(EndElement { name }));
            }

            b'?' => {
                // <?: processing instruction
                let Some(target) = self.name() else {
                    return Err(self.err_or("expected target name after <?"));
                };
                self.space();
                self.buf.clear();
                let mut b0 = 0u8;
                loop {
                    let Some(b) = self.mustgetc() else {
                        return Err(self.err.clone().expect("an error set"));
                    };
                    self.buf.push(b);
                    if b0 == b'?' && b == b'>' {
                        break;
                    }
                    b0 = b;
                }
                let data = self.buf[..self.buf.len() - 2].to_vec(); // chop ?>

                if target == "xml" {
                    let ver = proc_inst("version", &data);
                    if !ver.is_empty() && ver != b"1.0" {
                        let e = Error::Other(format!(
                            "xml: unsupported version {}; only version 1.0 is supported",
                            runui_core::golang::quote(&String::from_utf8_lossy(ver))
                        ));
                        self.err = Some(e.clone());
                        return Err(e);
                    }
                    let enc = proc_inst("encoding", &data);
                    if !enc.is_empty() && !enc.eq_ignore_ascii_case(b"utf-8") {
                        let e = Error::Other(format!(
                            "xml: encoding {} declared but Decoder.CharsetReader is nil",
                            runui_core::golang::quote(&String::from_utf8_lossy(enc))
                        ));
                        self.err = Some(e.clone());
                        return Err(e);
                    }
                }
                return Ok(Token::ProcInst(ProcInst { target, inst: data }));
            }

            b'!' => {
                // <!: maybe a comment, maybe CDATA
                let Some(nb) = self.mustgetc() else {
                    return Err(self.err.clone().expect("an error set"));
                };
                b = nb;
                match b {
                    b'-' => {
                        // <!-: probably <!-- for a comment
                        let Some(b) = self.mustgetc() else {
                            return Err(self.err.clone().expect("an error set"));
                        };
                        if b != b'-' {
                            return Err(self.set_syntax_error("invalid sequence <!- not part of <!--"));
                        }
                        // look for the end
                        self.buf.clear();
                        let (mut b0, mut b1) = (0u8, 0u8);
                        loop {
                            let Some(b) = self.mustgetc() else {
                                return Err(self.err.clone().expect("an error set"));
                            };
                            self.buf.push(b);
                            if b0 == b'-' && b1 == b'-' {
                                if b != b'>' {
                                    return Err(
                                        self.set_syntax_error("invalid sequence \"--\" not allowed in comments")
                                    );
                                }
                                break;
                            }
                            (b0, b1) = (b1, b);
                        }
                        let data = self.buf[..self.buf.len() - 3].to_vec(); // chop -->
                        return Ok(Token::Comment(data));
                    }
                    b'[' => {
                        // <![: probably <![CDATA[
                        for &c in b"CDATA[" {
                            let Some(b) = self.mustgetc() else {
                                return Err(self.err.clone().expect("an error set"));
                            };
                            if b != c {
                                return Err(self.set_syntax_error("invalid <![ sequence"));
                            }
                        }
                        // read the text until ]]>
                        let Some(data) = self.text(-1, true) else {
                            return Err(self.err.clone().expect("an error set"));
                        };
                        return Ok(Token::CharData(String::from_utf8(data).expect("checked")));
                    }
                    _ => {}
                }

                // Probably a directive: <!DOCTYPE ...>, <!ENTITY ...>, etc.,
                // kept for the caller. Quoted angle brackets don't count for
                // nesting.
                self.buf.clear();
                self.buf.push(b);
                let mut inquote = 0u8;
                let mut depth = 0i64;
                loop {
                    let Some(nb) = self.mustgetc() else {
                        return Err(self.err.clone().expect("an error set"));
                    };
                    b = nb;
                    if inquote == 0 && b == b'>' && depth == 0 {
                        break;
                    }
                    // Go's HandleB: b, and again for the byte that turned
                    // out not to start a comment
                    'handle: loop {
                        self.buf.push(b);
                        if b == inquote {
                            inquote = 0;
                        } else if inquote != 0 {
                            // in quotes, nothing special
                        } else if b == b'\'' || b == b'"' {
                            inquote = b;
                        } else if b == b'>' {
                            depth -= 1;
                        } else if b == b'<' {
                            // look for <!-- to begin a comment
                            let s = b"!--";
                            for i in 0..s.len() {
                                let Some(nb) = self.mustgetc() else {
                                    return Err(self.err.clone().expect("an error set"));
                                };
                                b = nb;
                                if b != s[i] {
                                    self.buf.extend_from_slice(&s[..i]);
                                    depth += 1;
                                    continue 'handle;
                                }
                            }
                            // remove the < written above
                            self.buf.pop();
                            // look for the end
                            let (mut b0, mut b1) = (0u8, 0u8);
                            loop {
                                let Some(nb) = self.mustgetc() else {
                                    return Err(self.err.clone().expect("an error set"));
                                };
                                b = nb;
                                if b0 == b'-' && b1 == b'-' && b == b'>' {
                                    break;
                                }
                                (b0, b1) = (b1, b);
                            }
                            // the comment is a space in the directive, so
                            // what it separated (a "<" and a "!") isn't
                            // joined into new meaning when it's written
                            // again
                            self.buf.push(b' ');
                        }
                        break;
                    }
                }
                return Ok(Token::Directive(self.buf.clone()));
            }
            _ => {}
        }

        // must be an open element like <a href="foo">
        self.ungetc(b);

        let Some(name) = self.nsname() else {
            return Err(self.err_or("expected element name after <"));
        };

        let mut attr: Vec<Attr> = Vec::new();
        let mut empty = false;
        loop {
            self.space();
            let Some(b) = self.mustgetc() else {
                return Err(self.err.clone().expect("an error set"));
            };
            if b == b'/' {
                empty = true;
                let Some(b) = self.mustgetc() else {
                    return Err(self.err.clone().expect("an error set"));
                };
                if b != b'>' {
                    return Err(self.set_syntax_error("expected /> in element"));
                }
                break;
            }
            if b == b'>' {
                break;
            }
            self.ungetc(b);

            let Some(aname) = self.nsname() else {
                return Err(self.err_or("expected attribute name in element"));
            };
            let mut a = Attr { name: aname, value: String::new() };
            self.space();
            let Some(b) = self.mustgetc() else {
                return Err(self.err.clone().expect("an error set"));
            };
            if b != b'=' {
                if self.strict {
                    return Err(self.set_syntax_error("attribute name without = in element"));
                }
                self.ungetc(b);
                a.value = a.name.local.clone();
            } else {
                self.space();
                let Some(data) = self.attrval() else {
                    return Err(self.err.clone().expect("an error set"));
                };
                a.value = String::from_utf8(data).expect("checked");
            }
            attr.push(a);
        }
        if empty {
            self.need_close = true;
            self.to_close = name.clone();
        }
        Ok(Token::StartElement(StartElement { name, attr }))
    }

    /// err_or is the error set, or else msg as a syntax error, set.
    fn err_or(&mut self, msg: &str) -> Error {
        match &self.err {
            Some(e) => e.clone(),
            None => self.set_syntax_error(msg),
        }
    }

    fn attrval(&mut self) -> Option<Vec<u8>> {
        let b = self.mustgetc()?;
        // quoted attribute values
        if b == b'"' || b == b'\'' {
            return self.text(b as i32, false);
        }
        // unquoted ones, refused when strict
        if self.strict {
            self.set_syntax_error("unquoted or missing attribute value in element");
            return None;
        }
        // and taken otherwise, the way HTML has them
        self.ungetc(b);
        self.buf.clear();
        loop {
            let b = self.mustgetc()?;
            if b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' {
                self.buf.push(b);
            } else {
                self.ungetc(b);
                break;
            }
        }
        Some(self.buf.clone())
    }

    /// space skips spaces, if any.
    fn space(&mut self) {
        while let Some(b) = self.getc() {
            if !matches!(b, b' ' | b'\r' | b'\n' | b'\t') {
                self.ungetc(b);
                return;
            }
        }
    }

    /// getc reads a byte, keeping the line number: None at the end or on
    /// an error, which is set.
    fn getc(&mut self) -> Option<u8> {
        if self.err.is_some() {
            return None;
        }
        let b = match self.next_byte.take() {
            Some(b) => b,
            None => {
                if self.pos >= self.data.len() {
                    self.err = Some(Error::Eof);
                    return None;
                }
                self.pos += 1;
                self.data[self.pos - 1]
            }
        };
        if b == b'\n' {
            self.line += 1;
            self.linestart = self.offset + 1;
        }
        self.offset += 1;
        Some(b)
    }

    /// mustgetc reads a byte that must be there: at the end, the error is
    /// a syntax error, "unexpected EOF".
    fn mustgetc(&mut self) -> Option<u8> {
        let b = self.getc();
        if b.is_none() && self.err == Some(Error::Eof) {
            self.err = Some(self.syntax_error("unexpected EOF"));
        }
        b
    }

    /// ungetc puts a byte back.
    fn ungetc(&mut self, b: u8) {
        if b == b'\n' {
            self.line -= 1;
        }
        self.next_byte = Some(b);
        self.offset -= 1;
    }

    /// text reads character data: up to a < or the end; in a quoted string
    /// (quote >= 0) up to the matching quote; in a CDATA section up to ]]>.
    /// Entities are replaced and \r and \r\n are \n. None on an error, set.
    fn text(&mut self, quote: i32, cdata: bool) -> Option<Vec<u8>> {
        let (mut b0, mut b1) = (0u8, 0u8);
        let mut trunc = 0;
        self.buf.clear();
        loop {
            let Some(mut b) = self.getc() else {
                if cdata {
                    if self.err == Some(Error::Eof) {
                        self.set_syntax_error("unexpected EOF in CDATA section");
                    }
                    return None;
                }
                break;
            };

            // A CDATA section ends with ]]>, an error in ordinary text but
            // allowed in quoted strings.
            if quote < 0 && b0 == b']' && b1 == b']' && b == b'>' {
                if cdata {
                    trunc = 2;
                    break;
                }
                self.set_syntax_error("unescaped ]]> not in CDATA section");
                return None;
            }

            // a < stops text
            if b == b'<' && !cdata {
                if quote >= 0 {
                    self.set_syntax_error("unescaped < inside quoted string");
                    return None;
                }
                self.ungetc(b'<');
                break;
            }
            if quote >= 0 && b as i32 == quote {
                break;
            }
            if b == b'&' && !cdata {
                // An entity, up to the semicolon. lt, gt, amp, apos and
                // quot are known without being declared, and those entity
                // has.
                let before = self.buf.len();
                self.buf.push(b'&');
                let mut text: Option<String> = None;
                b = self.mustgetc()?;
                if b == b'#' {
                    self.buf.push(b);
                    b = self.mustgetc()?;
                    let mut base = 10;
                    if b == b'x' {
                        base = 16;
                        self.buf.push(b);
                        b = self.mustgetc()?;
                    }
                    let start = self.buf.len();
                    while b.is_ascii_digit() || (base == 16 && b.is_ascii_hexdigit()) {
                        self.buf.push(b);
                        b = self.mustgetc()?;
                    }
                    if b != b';' {
                        self.ungetc(b);
                    } else {
                        let s = std::str::from_utf8(&self.buf[start..]).expect("digits").to_string();
                        self.buf.push(b';');
                        if let Ok(n) = u64::from_str_radix(&s, base)
                            && n <= 0x10FFFF
                        {
                            // Go's string(rune(n)): surrogates are U+FFFD
                            text = Some(char::from_u32(n as u32).unwrap_or('\u{FFFD}').to_string());
                        }
                    }
                } else {
                    self.ungetc(b);
                    if !self.read_name() && self.err.is_some() {
                        return None;
                    }
                    b = self.mustgetc()?;
                    if b != b';' {
                        self.ungetc(b);
                    } else {
                        let name = self.buf[before + 1..].to_vec();
                        self.buf.push(b';');
                        if is_name(&name) {
                            let s = String::from_utf8(name).expect("a name is UTF-8");
                            text = match s.as_str() {
                                "lt" => Some("<".to_string()),
                                "gt" => Some(">".to_string()),
                                "amp" => Some("&".to_string()),
                                "apos" => Some("'".to_string()),
                                "quot" => Some("\"".to_string()),
                                _ => self.entity.get(&s).cloned(),
                            };
                        }
                    }
                }

                if let Some(text) = text {
                    self.buf.truncate(before);
                    self.buf.extend_from_slice(text.as_bytes());
                    (b0, b1) = (0, 0);
                    continue;
                }
                if !self.strict {
                    (b0, b1) = (0, 0);
                    continue;
                }
                let mut ent = String::from_utf8_lossy(&self.buf[before..]).into_owned();
                if !ent.ends_with(';') {
                    ent.push_str(" (no semicolon)");
                }
                self.set_syntax_error(&format!("invalid character entity {ent}"));
                return None;
            }

            // unescaped \r and \r\n are \n
            if b == b'\r' {
                self.buf.push(b'\n');
            } else if b1 == b'\r' && b == b'\n' {
                // \r\n: the \n is written already
            } else {
                self.buf.push(b);
            }

            (b0, b1) = (b1, b);
        }
        let mut data = self.buf.clone();
        data.truncate(data.len() - trunc);

        // each rune must be one XML allows
        let mut rest = &data[..];
        while !rest.is_empty() {
            let (valid, bad) = match std::str::from_utf8(rest) {
                Ok(s) => (s, false),
                Err(e) => (std::str::from_utf8(&rest[..e.valid_up_to()]).expect("valid"), true),
            };
            if let Some(r) = valid.chars().find(|&r| !is_in_character_range(r)) {
                self.set_syntax_error(&format!("illegal character code U+{:04X}", r as u32));
                return None;
            }
            if bad {
                self.set_syntax_error("invalid UTF-8");
                return None;
            }
            rest = &rest[valid.len()..];
        }
        Some(data)
    }

    /// nsname reads a name with maybe a prefix and a colon: the prefix is
    /// the space. Two colons make no name, without an error set.
    fn nsname(&mut self) -> Option<Name> {
        let s = self.name()?;
        if s.matches(':').count() > 1 {
            return None;
        }
        match s.split_once(':') {
            Some((space, local)) if !space.is_empty() && !local.is_empty() => {
                Some(Name { space: space.to_string(), local: local.to_string() })
            }
            _ => Some(Name { space: String::new(), local: s }),
        }
    }

    /// name reads a name; when none is there no error is set (but the
    /// unexpected end), so the caller can say what was expected.
    fn name(&mut self) -> Option<String> {
        self.buf.clear();
        if !self.read_name() {
            return None;
        }
        // now the runes are checked
        if !is_name(&self.buf) {
            let msg = format!("invalid XML name: {}", String::from_utf8_lossy(&self.buf));
            self.set_syntax_error(&msg);
            return None;
        }
        Some(String::from_utf8(self.buf.clone()).expect("a name is UTF-8"))
    }

    /// read_name reads a name onto buf: up to a single-byte character that
    /// can't be in one. Every multi-byte character is taken; the caller
    /// checks them.
    fn read_name(&mut self) -> bool {
        let Some(b) = self.mustgetc() else {
            return false;
        };
        if b < 0x80 && !is_name_byte(b) {
            self.ungetc(b);
            return false;
        }
        self.buf.push(b);
        loop {
            let Some(b) = self.mustgetc() else {
                return false;
            };
            if b < 0x80 && !is_name_byte(b) {
                self.ungetc(b);
                break;
            }
            self.buf.push(b);
        }
        true
    }
}

/// equal_fold is Go's strings.EqualFold: equal under simple Unicode case
/// folding.
fn equal_fold(a: &str, b: &str) -> bool {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    a.chars().count() == b.chars().count() && a.chars().zip(b.chars()).all(|(x, y)| x == y || fold(x) == fold(y))
}

/// proc_inst is the value of param="..." or param='...' in a processing
/// instruction, or nothing; as rough as Go's, which works for all actual
/// cases.
fn proc_inst<'a>(param: &str, s: &'a [u8]) -> &'a [u8] {
    let param = format!("{param}=");
    let param = param.as_bytes();
    let lenp = param.len();
    let mut i = 0;
    let mut sep = 0u8;
    while i < s.len() {
        let sub = &s[i..];
        let k = match sub.windows(lenp).position(|w| w == param) {
            Some(k) if lenp + k < sub.len() => k,
            _ => return b"",
        };
        i += lenp + k + 1;
        let c = sub[lenp + k];
        if c == b'\'' || c == b'"' {
            sep = c;
            break;
        }
    }
    if sep == 0 {
        return b"";
    }
    match s[i..].iter().position(|&c| c == sep) {
        Some(j) => &s[i..i + j],
        None => b"",
    }
}

/// is_in_character_range says a rune is in XML's Char production.
fn is_in_character_range(r: char) -> bool {
    let r = r as u32;
    r == 0x09
        || r == 0x0A
        || r == 0x0D
        || (0x20..=0xD7FF).contains(&r)
        || (0xE000..=0xFFFD).contains(&r)
        || (0x10000..=0x10FFFF).contains(&r)
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'.' || c == b'-'
}

/// is_name says s is an XML name: its first rune in FIRST, the others in
/// FIRST or SECOND.
fn is_name(s: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(s) else {
        // Go stops at the first rune that isn't UTF-8: no name either way
        return false;
    };
    let mut chars = s.chars();
    let Some(c) = chars.next() else {
        return false;
    };
    if !in_table(FIRST, c) {
        return false;
    }
    chars.all(|c| in_table(FIRST, c) || in_table(SECOND, c))
}

/// in_table is Go's unicode.Is on a table of ranges with strides.
fn in_table(table: &[(u32, u32, u32)], r: char) -> bool {
    let r = r as u32;
    let i = table.partition_point(|&(_, hi, _)| hi < r);
    match table.get(i) {
        Some(&(lo, _, stride)) if lo <= r => stride == 1 || (r - lo).is_multiple_of(stride),
        _ => false,
    }
}

/// escape_text is Go's xml.EscapeText: s with what XML can't hold as it is
/// escaped (quotes, &, <, >, tab, new line and carriage return), and runes
/// outside XML's range U+FFFD.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in s.chars() {
        match r {
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ if !is_in_character_range(r) => out.push('\u{FFFD}'),
            _ => out.push(r),
        }
    }
    out
}

// These tables were generated by cut and paste from Appendix B of the XML
// spec at https://www.xml.com/axml/testaxml.htm and then reformatting, in
// Go; here they're Go's, (low, high, stride).

/// FIRST is (Letter | '_' | ':'), from Appendix B of the XML spec, as Go's first table has it.
const FIRST: &[(u32, u32, u32)] = &[
    (0x003A, 0x003A, 1),
    (0x0041, 0x005A, 1),
    (0x005F, 0x005F, 1),
    (0x0061, 0x007A, 1),
    (0x00C0, 0x00D6, 1),
    (0x00D8, 0x00F6, 1),
    (0x00F8, 0x00FF, 1),
    (0x0100, 0x0131, 1),
    (0x0134, 0x013E, 1),
    (0x0141, 0x0148, 1),
    (0x014A, 0x017E, 1),
    (0x0180, 0x01C3, 1),
    (0x01CD, 0x01F0, 1),
    (0x01F4, 0x01F5, 1),
    (0x01FA, 0x0217, 1),
    (0x0250, 0x02A8, 1),
    (0x02BB, 0x02C1, 1),
    (0x0386, 0x0386, 1),
    (0x0388, 0x038A, 1),
    (0x038C, 0x038C, 1),
    (0x038E, 0x03A1, 1),
    (0x03A3, 0x03CE, 1),
    (0x03D0, 0x03D6, 1),
    (0x03DA, 0x03E0, 2),
    (0x03E2, 0x03F3, 1),
    (0x0401, 0x040C, 1),
    (0x040E, 0x044F, 1),
    (0x0451, 0x045C, 1),
    (0x045E, 0x0481, 1),
    (0x0490, 0x04C4, 1),
    (0x04C7, 0x04C8, 1),
    (0x04CB, 0x04CC, 1),
    (0x04D0, 0x04EB, 1),
    (0x04EE, 0x04F5, 1),
    (0x04F8, 0x04F9, 1),
    (0x0531, 0x0556, 1),
    (0x0559, 0x0559, 1),
    (0x0561, 0x0586, 1),
    (0x05D0, 0x05EA, 1),
    (0x05F0, 0x05F2, 1),
    (0x0621, 0x063A, 1),
    (0x0641, 0x064A, 1),
    (0x0671, 0x06B7, 1),
    (0x06BA, 0x06BE, 1),
    (0x06C0, 0x06CE, 1),
    (0x06D0, 0x06D3, 1),
    (0x06D5, 0x06D5, 1),
    (0x06E5, 0x06E6, 1),
    (0x0905, 0x0939, 1),
    (0x093D, 0x093D, 1),
    (0x0958, 0x0961, 1),
    (0x0985, 0x098C, 1),
    (0x098F, 0x0990, 1),
    (0x0993, 0x09A8, 1),
    (0x09AA, 0x09B0, 1),
    (0x09B2, 0x09B2, 1),
    (0x09B6, 0x09B9, 1),
    (0x09DC, 0x09DD, 1),
    (0x09DF, 0x09E1, 1),
    (0x09F0, 0x09F1, 1),
    (0x0A05, 0x0A0A, 1),
    (0x0A0F, 0x0A10, 1),
    (0x0A13, 0x0A28, 1),
    (0x0A2A, 0x0A30, 1),
    (0x0A32, 0x0A33, 1),
    (0x0A35, 0x0A36, 1),
    (0x0A38, 0x0A39, 1),
    (0x0A59, 0x0A5C, 1),
    (0x0A5E, 0x0A5E, 1),
    (0x0A72, 0x0A74, 1),
    (0x0A85, 0x0A8B, 1),
    (0x0A8D, 0x0A8D, 1),
    (0x0A8F, 0x0A91, 1),
    (0x0A93, 0x0AA8, 1),
    (0x0AAA, 0x0AB0, 1),
    (0x0AB2, 0x0AB3, 1),
    (0x0AB5, 0x0AB9, 1),
    (0x0B05, 0x0B0C, 1),
    (0x0B0F, 0x0B10, 1),
    (0x0B13, 0x0B28, 1),
    (0x0B2A, 0x0B30, 1),
    (0x0B32, 0x0B33, 1),
    (0x0B36, 0x0B39, 1),
    (0x0B3D, 0x0B3D, 1),
    (0x0B5C, 0x0B5D, 1),
    (0x0B5F, 0x0B61, 1),
    (0x0B85, 0x0B8A, 1),
    (0x0B8E, 0x0B90, 1),
    (0x0B92, 0x0B95, 1),
    (0x0B99, 0x0B9A, 1),
    (0x0B9C, 0x0B9C, 1),
    (0x0B9E, 0x0B9F, 1),
    (0x0BA3, 0x0BA4, 1),
    (0x0BA8, 0x0BAA, 1),
    (0x0BAE, 0x0BB5, 1),
    (0x0BB7, 0x0BB9, 1),
    (0x0C05, 0x0C0C, 1),
    (0x0C0E, 0x0C10, 1),
    (0x0C12, 0x0C28, 1),
    (0x0C2A, 0x0C33, 1),
    (0x0C35, 0x0C39, 1),
    (0x0C60, 0x0C61, 1),
    (0x0C85, 0x0C8C, 1),
    (0x0C8E, 0x0C90, 1),
    (0x0C92, 0x0CA8, 1),
    (0x0CAA, 0x0CB3, 1),
    (0x0CB5, 0x0CB9, 1),
    (0x0CDE, 0x0CDE, 1),
    (0x0CE0, 0x0CE1, 1),
    (0x0D05, 0x0D0C, 1),
    (0x0D0E, 0x0D10, 1),
    (0x0D12, 0x0D28, 1),
    (0x0D2A, 0x0D39, 1),
    (0x0D60, 0x0D61, 1),
    (0x0E01, 0x0E2E, 1),
    (0x0E30, 0x0E30, 1),
    (0x0E32, 0x0E33, 1),
    (0x0E40, 0x0E45, 1),
    (0x0E81, 0x0E82, 1),
    (0x0E84, 0x0E84, 1),
    (0x0E87, 0x0E88, 1),
    (0x0E8A, 0x0E8D, 3),
    (0x0E94, 0x0E97, 1),
    (0x0E99, 0x0E9F, 1),
    (0x0EA1, 0x0EA3, 1),
    (0x0EA5, 0x0EA7, 2),
    (0x0EAA, 0x0EAB, 1),
    (0x0EAD, 0x0EAE, 1),
    (0x0EB0, 0x0EB0, 1),
    (0x0EB2, 0x0EB3, 1),
    (0x0EBD, 0x0EBD, 1),
    (0x0EC0, 0x0EC4, 1),
    (0x0F40, 0x0F47, 1),
    (0x0F49, 0x0F69, 1),
    (0x10A0, 0x10C5, 1),
    (0x10D0, 0x10F6, 1),
    (0x1100, 0x1100, 1),
    (0x1102, 0x1103, 1),
    (0x1105, 0x1107, 1),
    (0x1109, 0x1109, 1),
    (0x110B, 0x110C, 1),
    (0x110E, 0x1112, 1),
    (0x113C, 0x1140, 2),
    (0x114C, 0x1150, 2),
    (0x1154, 0x1155, 1),
    (0x1159, 0x1159, 1),
    (0x115F, 0x1161, 1),
    (0x1163, 0x1169, 2),
    (0x116D, 0x116E, 1),
    (0x1172, 0x1173, 1),
    (0x11AE, 0x11AF, 1),
    (0x11B7, 0x11B8, 1),
    (0x11BA, 0x11BA, 1),
    (0x11BC, 0x11C2, 1),
    (0x11F9, 0x11F9, 1),
    (0x1E00, 0x1E9B, 1),
    (0x1EA0, 0x1EF9, 1),
    (0x1F00, 0x1F15, 1),
    (0x1F18, 0x1F1D, 1),
    (0x1F20, 0x1F45, 1),
    (0x1F48, 0x1F4D, 1),
    (0x1F50, 0x1F57, 1),
    (0x1F5D, 0x1F5D, 1),
    (0x1F5F, 0x1F7D, 1),
    (0x1F80, 0x1FB4, 1),
    (0x1FB6, 0x1FBC, 1),
    (0x1FBE, 0x1FBE, 1),
    (0x1FC2, 0x1FC4, 1),
    (0x1FC6, 0x1FCC, 1),
    (0x1FD0, 0x1FD3, 1),
    (0x1FD6, 0x1FDB, 1),
    (0x1FE0, 0x1FEC, 1),
    (0x1FF2, 0x1FF4, 1),
    (0x1FF6, 0x1FFC, 1),
    (0x2126, 0x2126, 1),
    (0x212A, 0x212B, 1),
    (0x212E, 0x212E, 1),
    (0x2180, 0x2182, 1),
    (0x3007, 0x3007, 1),
    (0x3021, 0x3029, 1),
    (0x3041, 0x3094, 1),
    (0x30A1, 0x30FA, 1),
    (0x3105, 0x312C, 1),
    (0x4E00, 0x9FA5, 1),
    (0xAC00, 0xD7A3, 1),
];

/// SECOND is NameChar's other runes, as Go's second table has it.
const SECOND: &[(u32, u32, u32)] = &[
    (0x002D, 0x002E, 1),
    (0x0030, 0x0039, 1),
    (0x00B7, 0x00B7, 1),
    (0x02D0, 0x02D1, 1),
    (0x0300, 0x0345, 1),
    (0x0360, 0x0361, 1),
    (0x0387, 0x0387, 1),
    (0x0483, 0x0486, 1),
    (0x0591, 0x05A1, 1),
    (0x05A3, 0x05B9, 1),
    (0x05BB, 0x05BD, 1),
    (0x05BF, 0x05BF, 1),
    (0x05C1, 0x05C2, 1),
    (0x064B, 0x0652, 1),
    (0x0660, 0x0669, 1),
    (0x0670, 0x0670, 1),
    (0x06D6, 0x06DC, 1),
    (0x06DD, 0x06DF, 1),
    (0x06E0, 0x06E4, 1),
    (0x06E7, 0x06E8, 1),
    (0x06EA, 0x06ED, 1),
    (0x06F0, 0x06F9, 1),
    (0x0901, 0x0903, 1),
    (0x093C, 0x093C, 1),
    (0x093E, 0x094C, 1),
    (0x094D, 0x094D, 1),
    (0x0951, 0x0954, 1),
    (0x0962, 0x0963, 1),
    (0x0966, 0x096F, 1),
    (0x0981, 0x0983, 1),
    (0x09BC, 0x09BC, 1),
    (0x09BE, 0x09BF, 1),
    (0x09C0, 0x09C4, 1),
    (0x09C7, 0x09C8, 1),
    (0x09CB, 0x09CD, 1),
    (0x09D7, 0x09D7, 1),
    (0x09E2, 0x09E3, 1),
    (0x09E6, 0x09EF, 1),
    (0x0A3E, 0x0A3F, 1),
    (0x0A40, 0x0A42, 1),
    (0x0A47, 0x0A48, 1),
    (0x0A4B, 0x0A4D, 1),
    (0x0A66, 0x0A6F, 1),
    (0x0A70, 0x0A71, 1),
    (0x0A81, 0x0A83, 1),
    (0x0ABC, 0x0ABC, 1),
    (0x0ABE, 0x0AC5, 1),
    (0x0AC7, 0x0AC9, 1),
    (0x0ACB, 0x0ACD, 1),
    (0x0AE6, 0x0AEF, 1),
    (0x0B01, 0x0B03, 1),
    (0x0B3C, 0x0B3C, 1),
    (0x0B3E, 0x0B43, 1),
    (0x0B47, 0x0B48, 1),
    (0x0B4B, 0x0B4D, 1),
    (0x0B56, 0x0B57, 1),
    (0x0B66, 0x0B6F, 1),
    (0x0B82, 0x0B83, 1),
    (0x0BBE, 0x0BC2, 1),
    (0x0BC6, 0x0BC8, 1),
    (0x0BCA, 0x0BCD, 1),
    (0x0BD7, 0x0BD7, 1),
    (0x0BE7, 0x0BEF, 1),
    (0x0C01, 0x0C03, 1),
    (0x0C3E, 0x0C44, 1),
    (0x0C46, 0x0C48, 1),
    (0x0C4A, 0x0C4D, 1),
    (0x0C55, 0x0C56, 1),
    (0x0C66, 0x0C6F, 1),
    (0x0C82, 0x0C83, 1),
    (0x0CBE, 0x0CC4, 1),
    (0x0CC6, 0x0CC8, 1),
    (0x0CCA, 0x0CCD, 1),
    (0x0CD5, 0x0CD6, 1),
    (0x0CE6, 0x0CEF, 1),
    (0x0D02, 0x0D03, 1),
    (0x0D3E, 0x0D43, 1),
    (0x0D46, 0x0D48, 1),
    (0x0D4A, 0x0D4D, 1),
    (0x0D57, 0x0D57, 1),
    (0x0D66, 0x0D6F, 1),
    (0x0E31, 0x0E31, 1),
    (0x0E34, 0x0E3A, 1),
    (0x0E46, 0x0E46, 1),
    (0x0E47, 0x0E4E, 1),
    (0x0E50, 0x0E59, 1),
    (0x0EB1, 0x0EB1, 1),
    (0x0EB4, 0x0EB9, 1),
    (0x0EBB, 0x0EBC, 1),
    (0x0EC6, 0x0EC6, 1),
    (0x0EC8, 0x0ECD, 1),
    (0x0ED0, 0x0ED9, 1),
    (0x0F18, 0x0F19, 1),
    (0x0F20, 0x0F29, 1),
    (0x0F35, 0x0F39, 2),
    (0x0F3E, 0x0F3F, 1),
    (0x0F71, 0x0F84, 1),
    (0x0F86, 0x0F8B, 1),
    (0x0F90, 0x0F95, 1),
    (0x0F97, 0x0F97, 1),
    (0x0F99, 0x0FAD, 1),
    (0x0FB1, 0x0FB7, 1),
    (0x0FB9, 0x0FB9, 1),
    (0x20D0, 0x20DC, 1),
    (0x302A, 0x302F, 1),
    (0x3031, 0x3035, 1),
    (0x3099, 0x309A, 1),
    (0x309D, 0x309E, 1),
    (0x30FC, 0x30FE, 1),
];

#[cfg(test)]
mod tests {
    use super::*;
    use runui_core::golang::quote;

    /// tokens is every token of data, and how decoding ended, as the Go
    /// decoder's were printed to check these against.
    fn tokens(data: &str, strict: bool, raw: bool) -> Vec<String> {
        let mut d = Decoder::new(data.as_bytes());
        d.strict = strict;
        let mut out = Vec::new();
        loop {
            let t = if raw { d.raw_token() } else { d.token() };
            let (l, c) = d.input_pos();
            let at = format!("@{} {l}:{c}", d.input_offset());
            match t {
                Err(e) => {
                    out.push(format!("err {e} {at}"));
                    return out;
                }
                Ok(t) => out.push(format!("{} {at}", show(&t))),
            }
        }
    }

    fn show(t: &Token) -> String {
        let lossy = |b: &[u8]| quote(&String::from_utf8_lossy(b));
        match t {
            Token::StartElement(t) => {
                let a: Vec<String> =
                    t.attr.iter().map(|a| format!("{}|{}={}", a.name.space, a.name.local, quote(&a.value))).collect();
                format!("S {}|{} [{}]", t.name.space, t.name.local, a.join(" "))
            }
            Token::EndElement(t) => format!("E {}|{}", t.name.space, t.name.local),
            Token::CharData(c) => format!("C {}", quote(c)),
            Token::Comment(c) => format!("M {}", lossy(c)),
            Token::ProcInst(p) => format!("P {} {}", p.target, lossy(&p.inst)),
            Token::Directive(d) => format!("D {}", lossy(d)),
        }
    }

    // What Go's encoding/xml returned for the same documents, token for
    // token, with its offsets and positions.
    #[test]
    fn tokens_are_gos() {
        let ns = r#"<w:workspaceFilter xmlns:w="urn:w" xmlns="urn:d" xml:lang="en"><w:filter root="/a" w:mode="merge"><w:include pattern="/a(/.*)?"/><exclude pattern="/a/b"/><other pattern="x"/></w:filter><filter root="/b" mode="update" root="/c"><include pattern="1"><include pattern="2"/></include>text</filter></w:workspaceFilter>"#;
        assert_eq!(
            tokens(ns, true, false),
            [
                r#"S urn:w|workspaceFilter [xmlns|w="urn:w" |xmlns="urn:d" http://www.w3.org/XML/1998/namespace|lang="en"] @63 1:64"#,
                r#"S urn:w|filter [|root="/a" urn:w|mode="merge"] @98 1:99"#,
                r#"S urn:w|include [|pattern="/a(/.*)?"] @129 1:130"#,
                "E urn:w|include @129 1:130",
                r#"S urn:d|exclude [|pattern="/a/b"] @154 1:155"#,
                "E urn:d|exclude @154 1:155",
                r#"S urn:d|other [|pattern="x"] @174 1:175"#,
                "E urn:d|other @174 1:175",
                "E urn:w|filter @185 1:186",
                r#"S urn:d|filter [|root="/b" |mode="update" |root="/c"] @227 1:228"#,
                r#"S urn:d|include [|pattern="1"] @248 1:249"#,
                r#"S urn:d|include [|pattern="2"] @270 1:271"#,
                "E urn:d|include @270 1:271",
                "E urn:d|include @280 1:281",
                r#"C "text" @284 1:285"#,
                "E urn:d|filter @293 1:294",
                "E urn:w|workspaceFilter @313 1:314",
                "err EOF @313 1:314",
            ]
        );
        let lax = r#"<properties><entry key=name>unquoted</entry><entry key>valueless</entry><entry key="u">&unknown; & &#xZZ; &#; &lt</entry></properties>"#;
        assert_eq!(
            tokens(lax, false, false),
            [
                "S |properties [] @12 1:13",
                r#"S |entry [|key="name"] @28 1:29"#,
                r#"C "unquoted" @36 1:37"#,
                "E |entry @44 1:45",
                r#"S |entry [|key="key"] @55 1:56"#,
                r#"C "valueless" @64 1:65"#,
                "E |entry @72 1:73",
                r#"S |entry [|key="u"] @87 1:88"#,
                r#"C "&unknown; & &#xZZ; &#; &lt" @113 1:114"#,
                "E |entry @121 1:122",
                "E |properties @134 1:135",
                "err EOF @134 1:135",
            ]
        );
        let mismatch = r#"<properties><entry key="name">a</wrong><entry key="b">b</entry></properties>"#;
        assert_eq!(
            tokens(mismatch, false, false),
            [
                "S |properties [] @12 1:13",
                r#"S |entry [|key="name"] @30 1:31"#,
                r#"C "a" @31 1:32"#,
                "E |entry @39 1:40",
                "E |properties @39 1:40",
                "err XML syntax error on line 1: unexpected end element </wrong> @39 1:40",
            ]
        );
        let crlf = "<?xml version=\"1.0\"?>\r\n<properties>\r\n<entry key=\"name\">a\r\nb\rc</entry>\r\n<entry key=\"k\r\nv\">  spaced\t\n </entry>\r\n</properties>\r\n";
        assert_eq!(
            tokens(crlf, true, false),
            [
                r#"P xml "version=\"1.0\"" @21 1:22"#,
                r#"C "\n" @23 2:1"#,
                "S |properties [] @35 2:13",
                r#"C "\n" @37 3:1"#,
                r#"S |entry [|key="name"] @55 3:19"#,
                r#"C "a\nb\nc" @61 4:4"#,
                "E |entry @69 4:12",
                r#"C "\n" @71 5:1"#,
                r#"S |entry [|key="k\nv"] @89 6:4"#,
                r#"C "  spaced\t\n " @100 7:2"#,
                "E |entry @108 7:10",
                r#"C "\n" @110 8:1"#,
                "E |properties @123 8:14",
                r#"C "\n" @125 9:1"#,
                "err EOF @125 9:1",
            ]
        );
        let doctype = r#"<!DOCTYPE p [ <!ENTITY e "v"> <!-- a > comment --> <!ATTLIST a b CDATA "x>y"> ]><p><entry key="name">&e;</entry></p>"#;
        assert_eq!(
            tokens(doctype, true, true),
            [
                r#"D "DOCTYPE p [ <!ENTITY e \"v\">   <!ATTLIST a b CDATA \"x>y\"> ]" @80 1:81"#,
                "S |p [] @83 1:84",
                r#"S |entry [|key="name"] @101 1:102"#,
                "err XML syntax error on line 1: invalid character entity &e; @104 1:105",
            ]
        );
    }

    #[test]
    fn the_name_tables_are_sorted() {
        for table in [FIRST, SECOND] {
            assert!(table.windows(2).all(|w| w[0].1 < w[1].0), "ranges in order, apart");
        }
        assert!(is_name(b"jcr:primaryType") && is_name("prop\u{e9}rties".as_bytes()));
        assert!(!is_name(b"1a") && !is_name("x\u{2603}".as_bytes()) && !is_name(b""));
    }

    #[test]
    fn escape_text_is_gos() {
        assert_eq!(
            escape_text("a\"b'c&d<e>f\tg\nh\ri\u{1}j\u{fffe}k"),
            "a&#34;b&#39;c&amp;d&lt;e&gt;f&#x9;g&#xA;h&#xD;i\u{fffd}j\u{fffd}k"
        );
    }
}
