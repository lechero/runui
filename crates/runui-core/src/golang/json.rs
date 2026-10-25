//! Go's `encoding/json` as aem-cli's files and answers depend on it:
//! `json.Marshal`'s bytes (compact; <, >, & and U+2028, U+2029 escaped; a
//! raw value's strings kept as they came), a null leaving a field zero,
//! and the words of its commonest errors.

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize};

use super::quote::quote;

/// marshal_vec is v as JSON the way Go's `json.Marshal` writes it, as bytes.
pub fn marshal_vec<T: Serialize + ?Sized>(v: &T) -> serde_json::Result<Vec<u8>> {
    Ok(escape_compact(&serde_json::to_vec(v)?))
}

/// escape_compact writes JSON as Go's `json.Marshal` does: without
/// whitespace between tokens (a raw value may have some), and in strings
/// with <, >, & and U+2028, U+2029 escaped; everything else in a string as
/// it is.
pub fn escape_compact(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 16);
    let mut in_string = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_string {
            match c {
                b'\\' => {
                    out.extend_from_slice(&b[i..(i + 2).min(b.len())]);
                    i += 2;
                    continue;
                }
                b'"' => {
                    in_string = false;
                    out.push(c);
                }
                b'<' => out.extend_from_slice(b"\\u003c"),
                b'>' => out.extend_from_slice(b"\\u003e"),
                b'&' => out.extend_from_slice(b"\\u0026"),
                0xe2 if b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xa8 | 0xa9)) => {
                    out.extend_from_slice(if b[i + 2] == 0xa8 { b"\\u2028" } else { b"\\u2029" });
                    i += 3;
                    continue;
                }
                _ => out.push(c),
            }
        } else {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b'"' => {
                    in_string = true;
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        i += 1;
    }
    out
}

/// marshal_indent is v as JSON the way Go's `json.MarshalIndent(v, prefix,
/// indent)` writes it: `json.Marshal`'s bytes laid out by `json.Indent`.
pub fn marshal_indent<T: Serialize + ?Sized>(v: &T, prefix: &str, ind: &str) -> serde_json::Result<Vec<u8>> {
    let b = marshal_vec(v)?;
    Ok(indent(&b, prefix, ind).unwrap_or(b))
}

/// marshal is v as JSON the way Go's `json.Marshal` writes it, as text.
pub fn marshal<T: Serialize + ?Sized>(v: &T) -> serde_json::Result<String> {
    Ok(String::from_utf8(marshal_vec(v)?).expect("JSON is UTF-8"))
}

/// decode reads JSON into a value the way Go's `json.Unmarshal` fills one:
/// a null leaves it zero.
pub fn decode<T: DeserializeOwned + Default>(body: &[u8]) -> serde_json::Result<T> {
    Ok(serde_json::from_slice::<Option<T>>(body)?.unwrap_or_default())
}

/// json_error_words is why JSON didn't read, in Go's words where they're
/// known (nothing there, or no JSON from the start, like an HTML page),
/// else serde_json's.
pub fn json_error_words(body: &[u8], err: &serde_json::Error) -> String {
    let first = body.iter().copied().find(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r'));
    match first {
        None => "unexpected end of JSON input".to_string(),
        Some(c) if !matches!(c, b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n') => {
            format!("invalid character {} looking for beginning of value", quote_char(c))
        }
        _ if err.is_eof() => "unexpected end of JSON input".to_string(),
        _ => err.to_string(),
    }
}

/// type_error_words is a value of the wrong type in JSON read into an
/// anonymous struct, in Go's words: `json: cannot unmarshal string into Go
/// struct field .schema of type bool`, or `into .kinds.0` for an element of
/// a list. Only for fields whose Go types it can name, bool, string and
/// int; None for the rest, and for any other error.
pub fn type_error_words(body: &[u8], err: &serde_json::Error) -> Option<String> {
    let msg = err.to_string();
    let rest = msg.strip_prefix("invalid type: ")?;
    let (unexpected, rest) = rest.split_once(", expected ")?;
    let expected = rest.rsplit_once(" at line ").map_or(rest, |(e, _)| e);
    let go_type = match expected {
        "a boolean" => "bool",
        "a string" => "string",
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" | "an integer" => "int",
        _ => return None,
    };
    let kind = match unexpected.split(' ').next().unwrap_or("") {
        "string" => "string",
        "boolean" => "bool",
        "integer" | "floating" => "number",
        "map" => "object",
        "sequence" => "array",
        _ => return None,
    };
    let (path, in_list, literal) = value_before(body, err.line(), err.column())?;
    // a number that isn't an int is named, as Go names it
    let value = if kind == "number" && go_type == "int" { format!("number {literal}") } else { kind.to_string() };
    let into = if in_list { "" } else { "Go struct field " };
    Some(format!("json: cannot unmarshal {value} into {into}.{path} of type {go_type}"))
}

/// value_before is the last value JSON starts before a line and column
/// (serde_json's, where it found what's wrong): its path, dotted, whether
/// it's an element of a list, and its text when it's a number or a word.
fn value_before(body: &[u8], line: usize, column: usize) -> Option<(String, bool, String)> {
    let line_start = if line <= 1 {
        0
    } else {
        body.iter().enumerate().filter(|(_, c)| **c == b'\n').nth(line - 2).map(|(i, _)| i + 1)?
    };
    // a scalar ends just before the column, a list or an object starts at it
    let end = (line_start + column).min(body.len().saturating_sub(1));
    enum Frame {
        Object(Option<String>),
        List(usize),
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut last: Option<(String, bool, String)> = None;
    let here = |stack: &[Frame]| -> (String, bool) {
        let parts: Vec<String> = stack
            .iter()
            .map(|f| match f {
                Frame::Object(k) => k.clone().unwrap_or_default(),
                Frame::List(n) => n.to_string(),
            })
            .collect();
        (parts.join("."), matches!(stack.last(), Some(Frame::List(_))))
    };
    let mut i = 0;
    while i <= end {
        match body[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b':' => i += 1,
            b'{' | b'[' => {
                let (p, in_list) = here(&stack);
                last = Some((p, in_list, String::new()));
                stack.push(if body[i] == b'{' { Frame::Object(None) } else { Frame::List(0) });
                i += 1;
            }
            b'}' | b']' => {
                stack.pop();
                i += 1;
            }
            b',' => {
                match stack.last_mut() {
                    Some(Frame::List(n)) => *n += 1,
                    Some(Frame::Object(k)) => *k = None,
                    None => {}
                }
                i += 1;
            }
            b'"' => {
                let mut j = i + 1;
                while j < body.len() && body[j] != b'"' {
                    j += if body[j] == b'\\' { 2 } else { 1 };
                }
                let text = String::from_utf8_lossy(&body[i + 1..j.min(body.len())]).into_owned();
                match stack.last_mut() {
                    Some(Frame::Object(k @ None)) => *k = Some(text),
                    _ => {
                        let (p, in_list) = here(&stack);
                        last = Some((p, in_list, String::new()));
                    }
                }
                i = j + 1;
            }
            _ => {
                let mut j = i;
                while j < body.len() && !matches!(body[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                let (p, in_list) = here(&stack);
                last = Some((p, in_list, String::from_utf8_lossy(&body[i..j]).into_owned()));
                i = j;
            }
        }
    }
    last.filter(|(p, ..)| !p.is_empty())
}

/// quote_char is a byte in single quotes, as Go's JSON errors quote one.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".to_string(),
        b'"' => "'\"'".to_string(),
        _ => {
            let q = quote(&char::from(c).to_string());
            format!("'{}'", &q[1..q.len() - 1])
        }
    }
}

/// nullable reads a field that may be null as its zero value, as Go's
/// decoder leaves a field a null is given for:
/// `#[serde(default, deserialize_with = "aem_core::golang::nullable")]`.
pub fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// unmarshal reads JSON into a struct or a map the way Go's
/// `json.Unmarshal` fills one: a null leaves it zero, a field's null leaves
/// the field zero (with `nullable`), and anything but an object is an
/// error (serde would read a struct from a list).
pub fn unmarshal<T: DeserializeOwned + Default>(body: &[u8]) -> Result<T, serde_json::Error> {
    let first = body.iter().copied().find(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r'));
    if first == Some(b'{') {
        return serde_json::from_slice(body);
    }
    match serde_json::from_slice::<Option<IgnoredAny>>(body)? {
        None => Ok(T::default()),
        Some(_) => Err(serde::de::Error::custom("cannot unmarshal a value other than an object")),
    }
}

/// valid reports whether b is JSON: Go's `json.Valid`.
pub fn valid(b: &[u8]) -> bool {
    serde_json::from_slice::<IgnoredAny>(b).is_ok()
}

/// indent is JSON laid out the way Go's `json.Indent` lays it out: each
/// element of an object or a list on a line of its own, after prefix and
/// as many indents as it's deep, a space after each colon, an empty object
/// or list kept as {} or []; strings and numbers as they were written, the
/// space before the value dropped and the space after it kept. None when
/// it isn't JSON.
pub fn indent(src: &[u8], prefix: &str, indent: &str) -> Option<Vec<u8>> {
    if !valid(src) {
        return None;
    }
    let is_space = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r');
    let end = src.len() - src.iter().rev().take_while(|c| is_space(c)).count();
    let mut dst = Vec::with_capacity(src.len() * 2);
    let newline = |dst: &mut Vec<u8>, depth: usize| {
        dst.push(b'\n');
        dst.extend_from_slice(prefix.as_bytes());
        for _ in 0..depth {
            dst.extend_from_slice(indent.as_bytes());
        }
    };
    let (mut need_indent, mut depth) = (false, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for &c in &src[..end] {
        if in_string {
            dst.push(c);
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        if is_space(&c) {
            continue;
        }
        if need_indent && c != b'}' && c != b']' {
            need_indent = false;
            depth += 1;
            newline(&mut dst, depth);
        }
        match c {
            b'"' => {
                in_string = true;
                dst.push(c);
            }
            // the indent waits, so an empty object or list stays {} or []
            b'{' | b'[' => {
                need_indent = true;
                dst.push(c);
            }
            b',' => {
                dst.push(c);
                newline(&mut dst, depth);
            }
            b':' => dst.extend_from_slice(b": "),
            b'}' | b']' => {
                if need_indent {
                    need_indent = false;
                } else {
                    depth = depth.saturating_sub(1);
                    newline(&mut dst, depth);
                }
                dst.push(c);
            }
            _ => dst.push(c),
        }
    }
    dst.extend_from_slice(&src[end..]);
    Some(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go 1.27's words, for an anonymous struct of these fields
    #[test]
    fn type_errors_are_worded_as_gos() {
        #[derive(Default, Deserialize, Debug)]
        #[serde(default)]
        #[allow(dead_code)]
        struct Args {
            schema: bool,
            depth: Option<i64>,
            limit: usize,
            name: String,
            kinds: Vec<String>,
            predicates: std::collections::BTreeMap<String, String>,
            paths: Vec<Path>,
        }
        #[derive(Default, Deserialize, Debug)]
        #[serde(default)]
        #[allow(dead_code)]
        struct Path {
            path: String,
            only: bool,
        }
        let words = |raw: &str| {
            let err = unmarshal::<Args>(raw.as_bytes()).unwrap_err();
            type_error_words(raw.as_bytes(), &err)
        };
        let go = |s: &str| Some(s.to_string());
        assert_eq!(
            words(r#"{"schema":"global"}"#),
            go("json: cannot unmarshal string into Go struct field .schema of type bool")
        );
        assert_eq!(
            words(r#"{"depth":"x"}"#),
            go("json: cannot unmarshal string into Go struct field .depth of type int")
        );
        assert_eq!(
            words(r#"{"limit":1.5}"#),
            go("json: cannot unmarshal number 1.5 into Go struct field .limit of type int")
        );
        assert_eq!(
            words(r#"{"limit": true}"#),
            go("json: cannot unmarshal bool into Go struct field .limit of type int")
        );
        assert_eq!(
            words(r#"{"limit":99999999999999999999}"#),
            go("json: cannot unmarshal number 99999999999999999999 into Go struct field .limit of type int")
        );
        assert_eq!(
            words(r#"{"x":[1,{"a":2}], "name":5}"#),
            go("json: cannot unmarshal number into Go struct field .name of type string")
        );
        assert_eq!(
            words(r#"{"name":{}}"#),
            go("json: cannot unmarshal object into Go struct field .name of type string")
        );
        assert_eq!(
            words("{\n  \"name\": [1]\n}"),
            go("json: cannot unmarshal array into Go struct field .name of type string")
        );
        assert_eq!(words(r#"{"kinds":[1]}"#), go("json: cannot unmarshal number into .kinds.0 of type string"));
        assert_eq!(
            words(r#"{"predicates":{"type":1}}"#),
            go("json: cannot unmarshal number into Go struct field .predicates.type of type string")
        );
        assert_eq!(
            words(r#"{"paths":[{"path":"/a"},{"path":1}]}"#),
            go("json: cannot unmarshal number into Go struct field .paths.1.path of type string")
        );
        // Go names its composite types in ways only it knows
        assert_eq!(words(r#"{"kinds":"pages"}"#), None);
        assert_eq!(words(r#"{"paths":["x"]}"#), None);
    }

    #[test]
    fn json_is_compact_and_escaped() {
        let raw = serde_json::value::RawValue::from_string("{ \"a\" : [ 1 , \"<&>\" ] }".to_string()).unwrap();
        let out = marshal_vec(&(raw, "x<y\u{2028}")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "[{\"a\":[1,\"\\u003c\\u0026\\u003e\"]},\"x\\u003cy\\u2028\"]");
    }

    #[test]
    fn null_is_zero() {
        #[derive(Default, Deserialize)]
        struct S {
            #[serde(default, deserialize_with = "nullable")]
            a: String,
            #[serde(default, deserialize_with = "nullable")]
            n: Vec<i64>,
        }
        let s: S = decode(br#"{"a":null,"n":null}"#).unwrap();
        assert_eq!((s.a.as_str(), s.n.len()), ("", 0));
        let s: S = decode(b"null").unwrap();
        assert_eq!(s.a, "");
        let err = decode::<S>(b"<html>").err().unwrap();
        assert_eq!(json_error_words(b"<html>", &err), "invalid character '<' looking for beginning of value");
        let err = decode::<S>(b"  ").err().unwrap();
        assert_eq!(json_error_words(b"  ", &err), "unexpected end of JSON input");
        let err = decode::<S>(br#"{"a":"#).err().unwrap();
        assert_eq!(json_error_words(br#"{"a":"#, &err), "unexpected end of JSON input");
    }

    #[test]
    fn only_objects_fill_a_struct() {
        #[derive(Default, Deserialize)]
        struct S {
            #[serde(default)]
            a: i64,
        }
        assert_eq!(unmarshal::<S>(br#"{"a":1}"#).unwrap().a, 1);
        assert_eq!(unmarshal::<S>(b"null").unwrap().a, 0);
        assert!(unmarshal::<S>(b"[1]").is_err());
    }

    #[test]
    fn json_is_indented_as_gos_is() {
        let src = br#"  {"a" : [1, 2.50, {}], "b":{"c":"x, y: {z}"},"d":[]} "#;
        let want = "{\n  \"a\": [\n    1,\n    2.50,\n    {}\n  ],\n  \"b\": {\n    \"c\": \"x, y: {z}\"\n  },\n  \"d\": []\n} ";
        assert_eq!(String::from_utf8(indent(src, "", "  ").unwrap()).unwrap(), want);
        assert_eq!(indent(b"\"a\\\"b\"", ">", "\t").unwrap(), b"\"a\\\"b\"");
        assert!(indent(b"{", "", "  ").is_none());
        assert!(valid(b"[1]") && !valid(b"") && !valid(b"<html>"));
    }
}
