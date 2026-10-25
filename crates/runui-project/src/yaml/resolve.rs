//! resolve.go: what a plain scalar is (a string, a bool, a number, null, a
//! timestamp) by yaml.v3's rules, Go's number and time parsing with them.

use std::sync::LazyLock;

use regex::Regex;

use super::YamlError;

pub(super) const NULL_TAG: &str = "!!null";
pub(super) const BOOL_TAG: &str = "!!bool";
pub(super) const STR_TAG: &str = "!!str";
pub(super) const INT_TAG: &str = "!!int";
pub(super) const FLOAT_TAG: &str = "!!float";
pub(super) const TIMESTAMP_TAG: &str = "!!timestamp";
pub(super) const SEQ_TAG: &str = "!!seq";
pub(super) const MAP_TAG: &str = "!!map";
pub(super) const BINARY_TAG: &str = "!!binary";
pub(super) const MERGE_TAG: &str = "!!merge";

const LONG_TAG_PREFIX: &str = "tag:yaml.org,2002:";

const SHORT_TAGS: [&str; 10] =
    [NULL_TAG, BOOL_TAG, STR_TAG, INT_TAG, FLOAT_TAG, TIMESTAMP_TAG, SEQ_TAG, MAP_TAG, BINARY_TAG, MERGE_TAG];

/// Resolved is what a scalar resolves to, as Go's resolve returns it in an
/// interface{}: the kinds the decoder tells apart.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Resolved {
    Null,
    Bool(bool),
    // Go's int or int64
    Int(i64),
    // Go's uint64
    Uint(u64),
    Float(f64),
    // a !!str, !!binary (decoded) or !!merge value, or a tag of its own's
    Str(String),
    // a time.Time; only that it is one matters here
    Timestamp(String),
}

pub(super) fn short_tag(tag: &str) -> String {
    if let Some(rest) = tag.strip_prefix(LONG_TAG_PREFIX) {
        for stag in SHORT_TAGS {
            if &stag[2..] == rest {
                return stag.to_string();
            }
        }
        return format!("!!{rest}");
    }
    tag.to_string()
}

pub(super) fn long_tag(tag: &str) -> String {
    if let Some(rest) = tag.strip_prefix("!!") {
        return format!("{LONG_TAG_PREFIX}{rest}");
    }
    tag.to_string()
}

fn resolvable_tag(tag: &str) -> bool {
    matches!(tag, "" | STR_TAG | BOOL_TAG | INT_TAG | FLOAT_TAG | NULL_TAG | TIMESTAMP_TAG)
}

static YAML_STYLE_FLOAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$").expect("a valid pattern"));

// The resolve table: what a scalar's first byte hints it might be.
fn resolve_hint(c: u8) -> u8 {
    match c {
        b'+' | b'-' => b'S',                                                                // Sign
        b'0'..=b'9' => b'D',                                                                // Digit
        b'y' | b'Y' | b'n' | b'N' | b't' | b'T' | b'f' | b'F' | b'o' | b'O' | b'~' => b'M', // In map
        b'.' => b'.', // Float (potentially in map)
        _ => 0,
    }
}

// The resolve map: the scalars that are what they are by their text.
fn resolve_map(s: &str) -> Option<(&'static str, Resolved)> {
    Some(match s {
        "true" | "True" | "TRUE" => (BOOL_TAG, Resolved::Bool(true)),
        "false" | "False" | "FALSE" => (BOOL_TAG, Resolved::Bool(false)),
        "" | "~" | "null" | "Null" | "NULL" => (NULL_TAG, Resolved::Null),
        ".nan" | ".NaN" | ".NAN" => (FLOAT_TAG, Resolved::Float(f64::NAN)),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => (FLOAT_TAG, Resolved::Float(f64::INFINITY)),
        "-.inf" | "-.Inf" | "-.INF" => (FLOAT_TAG, Resolved::Float(f64::NEG_INFINITY)),
        "<<" => (MERGE_TAG, Resolved::Str("<<".to_string())),
        _ => return None,
    })
}

/// resolve is what a scalar with a tag (or none) is: its tag and value. An
/// explicit tag the text can't be is an error.
pub(super) fn resolve(tag: &str, input: &str) -> Result<(String, Resolved), YamlError> {
    let tag = short_tag(tag);
    if !resolvable_tag(&tag) {
        return Ok((tag, Resolved::Str(input.to_string())));
    }

    let (rtag, out) = resolve_plain(&tag, input);

    match tag.as_str() {
        t if t.is_empty() || t == rtag || t == STR_TAG || t == BINARY_TAG => return Ok((rtag.to_string(), out)),
        FLOAT_TAG if rtag == INT_TAG => {
            if let Resolved::Int(v) = out {
                return Ok((FLOAT_TAG.to_string(), Resolved::Float(v as f64)));
            }
        }
        _ => {}
    }
    Err(YamlError(format!("yaml: cannot decode {} `{}` as a {}", short_tag(rtag), input, short_tag(&tag))))
}

fn resolve_plain(tag: &str, input: &str) -> (&'static str, Resolved) {
    // Any data is accepted as a !!str or !!binary.
    // Otherwise, the prefix is enough of a hint about what it might be.
    let hint = if input.is_empty() { b'N' } else { resolve_hint(input.as_bytes()[0]) };
    if hint != 0 && tag != STR_TAG && tag != BINARY_TAG {
        // Handle things we can lookup in a map.
        if let Some(item) = resolve_map(input) {
            return item;
        }

        // Base 60 floats are a bad idea, were dropped in YAML 1.2, and
        // are purposefully unsupported here. They're still quoted on
        // the way out for compatibility with other parser, though.

        match hint {
            b'M' => {
                // We've already checked the map above.
            }

            b'.' => {
                // Not in the map, so maybe a normal float.
                if let Some(f) = parse_float(input) {
                    return (FLOAT_TAG, Resolved::Float(f));
                }
            }

            b'D' | b'S' => {
                // Int, float, or timestamp.
                // Only try values as a timestamp if the value is unquoted or there's an explicit
                // !!timestamp tag.
                if (tag.is_empty() || tag == TIMESTAMP_TAG) && parse_timestamp(input) {
                    return (TIMESTAMP_TAG, Resolved::Timestamp(input.to_string()));
                }

                let plain = input.replace('_', "");
                if let Some(v) = parse_int(&plain, 0) {
                    return (INT_TAG, Resolved::Int(v));
                }
                if let Some(v) = parse_uint(&plain, 0) {
                    return (INT_TAG, Resolved::Uint(v));
                }
                if YAML_STYLE_FLOAT.is_match(&plain)
                    && let Some(f) = parse_float(&plain)
                {
                    return (FLOAT_TAG, Resolved::Float(f));
                }
                if let Some(rest) = plain.strip_prefix("0b") {
                    if let Some(v) = parse_int(rest, 2) {
                        return (INT_TAG, Resolved::Int(v));
                    }
                    if let Some(v) = parse_uint(rest, 2) {
                        return (INT_TAG, Resolved::Uint(v));
                    }
                } else if let Some(rest) = plain.strip_prefix("-0b")
                    && let Some(v) = parse_int(&format!("-{rest}"), 2)
                {
                    return (INT_TAG, Resolved::Int(v));
                }
                // Octals as introduced in version 1.2 of the spec.
                // Octals from the 1.1 spec, spelled as 0777, are still
                // decoded by default in v3 as well for compatibility.
                // May be dropped in v4 depending on how usage evolves.
                if let Some(rest) = plain.strip_prefix("0o") {
                    if let Some(v) = parse_int(rest, 8) {
                        return (INT_TAG, Resolved::Int(v));
                    }
                    if let Some(v) = parse_uint(rest, 8) {
                        return (INT_TAG, Resolved::Uint(v));
                    }
                } else if let Some(rest) = plain.strip_prefix("-0o")
                    && let Some(v) = parse_int(&format!("-{rest}"), 8)
                {
                    return (INT_TAG, Resolved::Int(v));
                }
            }
            _ => panic!("internal error: missing handler for resolver table: {} (with {input})", char::from(hint)),
        }
    }
    (STR_TAG, Resolved::Str(input.to_string()))
}

/// parse_uint is Go's strconv.ParseUint(s, base, 64), None for an error.
pub(super) fn parse_uint(s: &str, base: u32) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let b = s.as_bytes();
    let base0 = base == 0;
    let (mut base, mut digits) = (base, b);
    if base == 0 {
        // Look for octal, hex prefix.
        base = 10;
        if b[0] == b'0' {
            if b.len() >= 3 && b[1].eq_ignore_ascii_case(&b'b') {
                base = 2;
                digits = &b[2..];
            } else if b.len() >= 3 && b[1].eq_ignore_ascii_case(&b'o') {
                base = 8;
                digits = &b[2..];
            } else if b.len() >= 3 && b[1].eq_ignore_ascii_case(&b'x') {
                base = 16;
                digits = &b[2..];
            } else {
                base = 8;
                digits = &b[1..];
            }
        }
    }
    let mut n: u64 = 0;
    let mut underscores = false;
    for &c in digits {
        let d = match c {
            b'_' if base0 => {
                underscores = true;
                continue;
            }
            b'0'..=b'9' => c - b'0',
            c if c.to_ascii_lowercase().is_ascii_lowercase() => c.to_ascii_lowercase() - b'a' + 10,
            _ => return None,
        };
        if u32::from(d) >= base {
            return None;
        }
        n = n.checked_mul(u64::from(base))?.checked_add(u64::from(d))?;
    }
    if underscores && !underscore_ok(s) {
        return None;
    }
    Some(n)
}

/// parse_int is Go's strconv.ParseInt(s, base, 64), None for an error.
pub(super) fn parse_int(s: &str, base: u32) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (neg, rest) = match s.as_bytes()[0] {
        b'+' => (false, &s[1..]),
        b'-' => (true, &s[1..]),
        _ => (false, s),
    };
    let un = parse_uint(rest, base)?;
    let cutoff: u64 = 1 << 63;
    if !neg && un >= cutoff {
        return None;
    }
    if neg && un > cutoff {
        return None;
    }
    Some(if neg { (un as i64).wrapping_neg() } else { un as i64 })
}

// underscore_ok reports whether the underscores in s are allowed, as Go's
// strconv sees them: only between digits or between a base prefix and a
// digit.
fn underscore_ok(s: &str) -> bool {
    // saw tracks the last character (class) we saw:
    // ^ for beginning of number,
    // 0 for a digit or base prefix,
    // _ for an underscore,
    // ! for none of the above.
    let mut saw = b'^';
    let mut i = 0;
    let mut s = s.as_bytes();

    // Optional sign.
    if !s.is_empty() && (s[0] == b'-' || s[0] == b'+') {
        s = &s[1..];
    }

    // Optional base prefix.
    let mut hex = false;
    if s.len() >= 2 && s[0] == b'0' && matches!(s[1].to_ascii_lowercase(), b'b' | b'o' | b'x') {
        i = 2;
        saw = b'0'; // base prefix counts as a digit for "underscore as digit separator"
        hex = s[1].eq_ignore_ascii_case(&b'x');
    }

    // Number proper.
    while i < s.len() {
        // Digits are always okay.
        if s[i].is_ascii_digit() || hex && (b'a'..=b'f').contains(&s[i].to_ascii_lowercase()) {
            saw = b'0';
            i += 1;
            continue;
        }
        // Underscore must follow digit.
        if s[i] == b'_' {
            if saw != b'0' {
                return false;
            }
            saw = b'_';
            i += 1;
            continue;
        }
        // Underscore must also be followed by digit.
        if saw == b'_' {
            return false;
        }
        // Saw non-digit, non-underscore.
        saw = b'!';
        i += 1;
    }
    saw != b'_'
}

/// parse_float is Go's strconv.ParseFloat(s, 64) for decimal numbers, None
/// for an error: a syntax error, or a number too big (Go's ErrRange). The
/// special words (inf, nan) and hexadecimal floats aren't read: resolve
/// never hands it one.
fn parse_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let (mut sawdot, mut sawdigits, mut underscores) = (false, false, false);
    while i < b.len() {
        match b[i] {
            b'_' => underscores = true,
            b'.' => {
                if sawdot {
                    break;
                }
                sawdot = true;
            }
            b'0'..=b'9' => sawdigits = true,
            _ => break,
        }
        i += 1;
    }
    if !sawdigits {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i >= b.len() {
            return None;
        }
        if b[i] == b'+' || b[i] == b'-' {
            i += 1;
        }
        if i >= b.len() || !b[i].is_ascii_digit() {
            return None;
        }
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            if b[i] == b'_' {
                underscores = true;
            }
            i += 1;
        }
    }
    if underscores && !underscore_ok(&s[..i]) {
        return None;
    }
    if i != b.len() {
        return None;
    }
    let f: f64 = s.replace('_', "").parse().ok()?;
    if f.is_infinite() {
        return None;
    }
    Some(f)
}

/// is_base60_float reports whether s is in base 60 notation as defined in
/// YAML 1.1.
///
/// The base 60 float notation in YAML 1.1 is a terrible idea and is unsupported
/// in YAML 1.2 and by this package, but these should be marshalled quoted for
/// the time being for compatibility with other parsers.
pub(super) fn is_base60_float(s: &str) -> bool {
    // From http://yaml.org/type/float.html, except the regular expression there
    // is bogus. In practice parsers do not enforce the "\.[0-9_]*" suffix.
    static BASE60_FLOAT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+(?:\.[0-9_]*)?$").expect("a valid pattern"));
    // Fast path.
    let Some(&c) = s.as_bytes().first() else {
        return false;
    };
    if !(c == b'+' || c == b'-' || c.is_ascii_digit()) || !s.contains(':') {
        return false;
    }
    // Do the full match.
    BASE60_FLOAT.is_match(s)
}

/// is_old_bool returns whether s is bool notation as defined in YAML 1.1.
///
/// We continue to force strings that YAML 1.1 would interpret as booleans to be
/// rendered as quotes strings so that the marshalled output valid for YAML 1.1
/// parsing.
pub(super) fn is_old_bool(s: &str) -> bool {
    matches!(
        s,
        "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" | "n" | "N" | "no" | "No" | "NO" | "off" | "Off" | "OFF"
    )
}

/// parse_timestamp reports whether s is a timestamp, as yaml.v3 reads one
/// with Go's time.Parse: one of
///
/// ```text
/// 2006-1-2T15:4:5.999999999Z07:00 (RFC 3339 with short date fields)
/// 2006-1-2t15:4:5.999999999Z07:00 (and a lower-case "t")
/// 2006-1-2 15:4:5.999999999 (space separated with no time zone)
/// 2006-1-2 (date only)
/// ```
fn parse_timestamp(s: &str) -> bool {
    // Quick check: all date formats start with YYYY-.
    let b = s.as_bytes();
    let i = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if i != 4 || i == b.len() || b[i] != b'-' {
        return false;
    }
    [Some(b'T'), Some(b't'), Some(b' '), None].into_iter().any(|sep| parse_time(b, sep))
}

// getnum is Go's time.getnum: one or two digits (two when fixed), and
// what's after them.
fn getnum(s: &[u8], fixed: bool) -> Option<(i64, &[u8])> {
    let digit = |i: usize| s.get(i).is_some_and(u8::is_ascii_digit);
    if !digit(0) {
        return None;
    }
    if !digit(1) {
        if fixed {
            return None;
        }
        return Some((i64::from(s[0] - b'0'), &s[1..]));
    }
    Some((i64::from(s[0] - b'0') * 10 + i64::from(s[1] - b'0'), &s[2..]))
}

// parse_time is time.Parse with one of parse_timestamp's layouts, its
// separator between the date and the time (None for the date alone; a
// space for the one without a zone): whether it parses.
fn parse_time(s: &[u8], sep: Option<u8>) -> bool {
    let year = s[..4].iter().fold(0i64, |n, c| n * 10 + i64::from(c - b'0'));
    let lit = |v: &[u8], c: u8| -> Option<usize> { (v.first() == Some(&c)).then_some(1) };
    let parse = || -> Option<()> {
        let mut v = &s[4..];
        v = &v[lit(v, b'-')?..];
        let (month, rest) = getnum(v, false)?;
        if !(1..=12).contains(&month) {
            return None;
        }
        v = &rest[lit(rest, b'-')?..];
        let (day, rest) = getnum(v, false)?;
        v = rest;
        if let Some(sep) = sep {
            if sep == b' ' {
                // a space in the layout takes any number of them
                if !v.is_empty() && v[0] != b' ' {
                    return None;
                }
                while v.first() == Some(&b' ') {
                    v = &v[1..];
                }
            } else {
                v = &v[lit(v, sep)?..];
            }
            let (hour, rest) = getnum(v, false)?;
            if !(0..24).contains(&hour) {
                return None;
            }
            v = &rest[lit(rest, b':')?..];
            let (min, rest) = getnum(v, false)?;
            if !(0..60).contains(&min) {
                return None;
            }
            v = &rest[lit(rest, b':')?..];
            let (sec, rest) = getnum(v, false)?;
            if !(0..60).contains(&sec) {
                return None;
            }
            v = rest;
            // .999999999: a fraction, if there's one
            if v.len() >= 2 && (v[0] == b'.' || v[0] == b',') && v[1].is_ascii_digit() {
                let n = 1 + v[1..].iter().take_while(|c| c.is_ascii_digit()).count();
                v = &v[n..];
            }
            if sep != b' ' {
                // Z07:00
                if v.first() == Some(&b'Z') {
                    v = &v[1..];
                } else {
                    if v.len() < 6 || v[3] != b':' {
                        return None;
                    }
                    let (hr, _) = getnum(&v[1..3], true)?;
                    let (mm, _) = getnum(&v[4..6], true)?;
                    if hr > 24 || mm > 60 {
                        return None;
                    }
                    if v[0] != b'+' && v[0] != b'-' {
                        return None;
                    }
                    v = &v[6..];
                }
            }
        }
        if !v.is_empty() {
            return None;
        }
        if day < 1 || day > days_in(month, year) {
            return None;
        }
        Some(())
    };
    parse().is_some()
}

fn days_in(month: i64, year: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_resolve_as_yaml_v3_resolves_them() {
        let tag = |s: &str| resolve("", s).unwrap().0;
        for (s, want) in [
            ("", NULL_TAG),
            ("~", NULL_TAG),
            ("true", BOOL_TAG),
            ("yes", STR_TAG),
            ("12", INT_TAG),
            ("-0x1F", INT_TAG),
            ("0o17", INT_TAG),
            ("0b101", INT_TAG),
            ("1_000", INT_TAG),
            ("18446744073709551615", INT_TAG),
            ("1.5", FLOAT_TAG),
            (".5", FLOAT_TAG),
            ("1e999", STR_TAG),
            (".inf", FLOAT_TAG),
            ("2026-10-24", TIMESTAMP_TAG),
            ("2026-02-30", STR_TAG),
            ("2026-1-2T3:4:5.123Z", TIMESTAMP_TAG),
            ("2026-1-2 03:04:05", TIMESTAMP_TAG),
            ("2026-1-2T03:04:05+01:00", TIMESTAMP_TAG),
            ("2026-1-2T03:04:05", STR_TAG),
            ("http://x", STR_TAG),
            // the parser tags a plain << as a merge, resolve doesn't
            ("<<", STR_TAG),
        ] {
            assert_eq!(tag(s), want, "{s:?}");
        }
        assert_eq!(resolve("!!int", "abc").unwrap_err().0, "yaml: cannot decode !!str `abc` as a !!int");
        assert_eq!(resolve("!!float", "3").unwrap(), (FLOAT_TAG.to_string(), Resolved::Float(3.0)));
        assert_eq!(resolve("!foo", "x").unwrap(), ("!foo".to_string(), Resolved::Str("x".to_string())));
        assert!(is_base60_float("1:20") && !is_base60_float("1:2a"));
    }
}
