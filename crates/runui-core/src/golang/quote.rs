//! Go's `strconv.Quote` and `strconv.QuoteRune`, what `%q` writes: errors
//! that name a method, a tool or a path the way the Go build's do, and Go's
//! JSON errors quote what they found the same way.

use super::print::{IS_NOT_PRINT16, IS_NOT_PRINT32, IS_PRINT16, IS_PRINT32};

const LOWERHEX: &[u8; 16] = b"0123456789abcdef";

/// is_print is Go's `strconv.IsPrint` (and `unicode.IsPrint`): letters,
/// marks, numbers, punctuation, symbols and the ASCII space.
pub fn is_print(r: char) -> bool {
    let r = r as u32;
    // Fast check for Latin-1
    if r <= 0xff {
        if (0x20..=0x7e).contains(&r) {
            // All the ASCII is printable from space through DEL-1.
            return true;
        }
        if (0xa1..=0xff).contains(&r) {
            // Similarly for ¡ through ÿ...
            return r != 0xad; // ...except for the bizarre soft hyphen.
        }
        return false;
    }

    // Same algorithm, either on u16 or u32 value.
    // First, find first i such that is_print[i] >= x.
    // This is the index of either the start or end of a pair that might span x.
    // The start is even (is_print[i&^1]) and the end is odd (is_print[i|1]).
    // If we find x in a range, make sure x is not in is_not_print list.
    if r < 1 << 16 {
        let rr = r as u16;
        let (i, _) = bsearch(IS_PRINT16, rr);
        if i >= IS_PRINT16.len() || rr < IS_PRINT16[i & !1] || IS_PRINT16[i | 1] < rr {
            return false;
        }
        let (_, found) = bsearch(IS_NOT_PRINT16, rr);
        return !found;
    }

    let (i, _) = bsearch(IS_PRINT32, r);
    if i >= IS_PRINT32.len() || r < IS_PRINT32[i & !1] || IS_PRINT32[i | 1] < r {
        return false;
    }
    if r >= 0x20000 {
        return true;
    }
    let (_, found) = bsearch(IS_NOT_PRINT32, (r - 0x10000) as u16);
    !found
}

/// bsearch is the first index whose value is at least v, and whether it is v.
fn bsearch<E: Copy + Ord>(s: &[E], v: E) -> (usize, bool) {
    let (mut i, mut j) = (0, s.len());
    while i < j {
        let h = i + (j - i) / 2;
        if s[h] < v {
            i = h + 1;
        } else {
            j = h;
        }
    }
    (i, i < s.len() && s[i] == v)
}

/// quote is s in double quotes, Go's `strconv.Quote` (and `%q`).
pub fn quote(s: &str) -> String {
    quote_bytes(s.as_bytes())
}

/// quote_bytes is `strconv.Quote` of bytes that may not be UTF-8: a byte
/// that isn't is written `\x` and its two hex digits.
pub fn quote_bytes(b: &[u8]) -> String {
    let mut buf = String::with_capacity(b.len() + 2);
    buf.push('"');
    let mut s = b;
    while !s.is_empty() {
        let (r, width) = decode_rune(s);
        match r {
            None => {
                buf.push_str("\\x");
                buf.push(LOWERHEX[(s[0] >> 4) as usize] as char);
                buf.push(LOWERHEX[(s[0] & 0xf) as usize] as char);
            }
            Some(r) => append_escaped_rune(&mut buf, r, '"'),
        }
        s = &s[width..];
    }
    buf.push('"');
    buf
}

/// quote_rune is r in single quotes, Go's `strconv.QuoteRune`.
pub fn quote_rune(r: char) -> String {
    let mut buf = String::with_capacity(4);
    buf.push('\'');
    append_escaped_rune(&mut buf, r, '\'');
    buf.push('\'');
    buf
}

fn append_escaped_rune(buf: &mut String, r: char, quote: char) {
    if r == quote || r == '\\' {
        // always backslashed
        buf.push('\\');
        buf.push(r);
        return;
    }
    if is_print(r) {
        buf.push(r);
        return;
    }
    match r {
        '\u{7}' => buf.push_str("\\a"),
        '\u{8}' => buf.push_str("\\b"),
        '\u{c}' => buf.push_str("\\f"),
        '\n' => buf.push_str("\\n"),
        '\r' => buf.push_str("\\r"),
        '\t' => buf.push_str("\\t"),
        '\u{b}' => buf.push_str("\\v"),
        _ => {
            let r = r as u32;
            if r < u32::from(b' ') || r == 0x7f {
                buf.push_str("\\x");
                buf.push(LOWERHEX[(r >> 4) as usize & 0xf] as char);
                buf.push(LOWERHEX[r as usize & 0xf] as char);
            } else if r < 0x10000 {
                buf.push_str("\\u");
                for s in [12, 8, 4, 0] {
                    buf.push(LOWERHEX[(r >> s) as usize & 0xf] as char);
                }
            } else {
                buf.push_str("\\U");
                for s in [28, 24, 20, 16, 12, 8, 4, 0] {
                    buf.push(LOWERHEX[(r >> s) as usize & 0xf] as char);
                }
            }
        }
    }
}

/// decode_rune is the first rune of b and its width, Go's
/// `utf8.DecodeRune`: None and a width of 1 when b doesn't start with one
/// (empty b has width 0).
pub fn decode_rune(b: &[u8]) -> (Option<char>, usize) {
    if b.is_empty() {
        return (None, 0);
    }
    let head = &b[..b.len().min(4)];
    match head.utf8_chunks().next() {
        Some(chunk) => match chunk.valid().chars().next() {
            Some(c) => (Some(c), c.len_utf8()),
            None => (None, 1),
        },
        None => (None, 0),
    }
}

/// go_lossy is bytes as text, each byte that doesn't start a character
/// U+FFFD, as Go's `utf8.DecodeRune` (and its JSON decoder) reads them.
pub fn go_lossy(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let (r, n) = decode_rune(&b[i..]);
        out.push(r.unwrap_or('\u{FFFD}'));
        i += n.max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_as_go_counts_it() {
        // Go 1.27: 159613 printable runes, and this sum over them
        let (mut n, mut sum) = (0u64, 0u64);
        for r in (0..=0x10ffffu32).filter_map(char::from_u32) {
            if is_print(r) {
                n += 1;
                sum += u64::from(r as u32) * u64::from(r as u32 % 7 + 1);
            }
        }
        assert_eq!((n, sum), (159613, 69041369385));
    }

    #[test]
    fn quotes_as_go_does() {
        assert_eq!(quote("nope"), "\"nope\"");
        assert_eq!(
            quote("a\"b\\c\n\t\u{1}\u{7f}\u{a0}é\u{200b}\u{feff}\u{2028}😀\u{fffd}"),
            "\"a\\\"b\\\\c\\n\\t\\x01\\x7f\\u00a0é\\u200b\\ufeff\\u2028😀\u{fffd}\""
        );
        assert_eq!(quote("\u{e0001}\u{f0000}\u{1d173}"), "\"\\U000e0001\\U000f0000\\U0001d173\"");
        assert_eq!(quote_bytes(b"a\xffb"), "\"a\\xffb\"");
        assert_eq!(quote_rune('\''), "'\\''");
        assert_eq!(quote_rune('\u{feff}'), "'\\ufeff'");
        assert_eq!(quote_rune('u'), "'u'");
    }
}
