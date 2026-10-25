//! The favorites file read as Go's encoding/json (v1, on Go 1.27's v2)
//! reads it into `struct { Folders map[string][]Favorite }`: a field
//! matched by its name in any case (strings.EqualFold), the last of one
//! given twice winning, a map given twice merged; a null leaving a field as
//! it was, a list's null a zero Favorite; fields it doesn't know skipped;
//! and a value of the wrong type said as Go says it, by its path: `json:
//! cannot unmarshal number into Go struct field fileData.folders.~1p.0.command
//! of type string`.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use runui_core::golang::{parse_time, quote, strings::equal_fold, zero_time};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::{Favorite, FileData};

/// Path is where a value is, as Go's errors name it: the struct's name,
/// then each step down, a name or a map's key escaped as a JSON pointer's
/// token, or a list's index.
#[derive(Clone)]
struct Path {
    steps: String,
    /// the last step is a list's index: Go leaves "Go struct field" out
    index: bool,
}

impl Path {
    fn root() -> Path {
        Path { steps: "fileData".into(), index: false }
    }

    fn key(&self, k: &str) -> Path {
        Path { steps: format!("{}.{}", self.steps, k.replace('~', "~0").replace('/', "~1")), index: false }
    }

    fn at(&self, i: usize) -> Path {
        Path { steps: format!("{}.{i}", self.steps), index: true }
    }

    /// wrong is a value of the wrong kind here, in Go's words.
    fn wrong<E: de::Error>(&self, kind: &str, go_type: &str) -> E {
        let field = if self.index { "" } else { "Go struct field " };
        E::custom(format!("json: cannot unmarshal {kind} into {field}{} of type {go_type}", self.steps))
    }
}

/// wrong_kinds implements the kinds of value a visitor doesn't take as
/// Go's type errors, each saying its kind; a null is None.
macro_rules! wrong_kinds {
    ($go_type:expr, except $($taken:ident),*) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Err(self.path().wrong("bool", $go_type))
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Err(self.path().wrong("number", $go_type))
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Err(self.path().wrong("number", $go_type))
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Err(self.path().wrong("number", $go_type))
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        wrong_kinds!(@ $go_type, $($taken),*);
    };
    (@ $go_type:expr, map) => {
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Err(self.path().wrong("string", $go_type))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(self.path().wrong("array", $go_type))
        }
    };
    (@ $go_type:expr, seq) => {
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Err(self.path().wrong("string", $go_type))
        }
        fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(self.path().wrong("object", $go_type))
        }
    };
    (@ $go_type:expr, str) => {
        fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(self.path().wrong("array", $go_type))
        }
        fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(self.path().wrong("object", $go_type))
        }
    };
}

impl<'de> serde::Deserialize<'de> for FileData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<FileData, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FileData;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<FileData, A::Error> {
                let mut out = FileData::default();
                while let Some(k) = m.next_key::<String>()? {
                    if equal_fold(&k, "folders") {
                        // a map decoded into one there already adds to it
                        if let Some(folders) = m.next_value_seed(Folders(Path::root().key(&k)))? {
                            out.folders.extend(folders);
                        }
                    } else {
                        m.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_map(V)
    }
}

/// Folders is fileData's map: each folder's list, a null one empty.
struct Folders(Path);

impl Folders {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl<'de> DeserializeSeed<'de> for Folders {
    type Value = Option<BTreeMap<String, Vec<Favorite>>>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Folders {
    type Value = Option<BTreeMap<String, Vec<Favorite>>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a map")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Self::Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(dir) = m.next_key::<String>()? {
            let favs = m.next_value_seed(List(self.0.key(&dir)))?.unwrap_or_default();
            out.insert(dir, favs);
        }
        Ok(Some(out))
    }
    wrong_kinds!("map[string][]favorites.Favorite", except map);
}

/// List is a folder's favorites, a null among them a zero Favorite.
struct List(Path);

impl List {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl<'de> DeserializeSeed<'de> for List {
    type Value = Option<Vec<Favorite>>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for List {
    type Value = Option<Vec<Favorite>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a sequence")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut s: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::new();
        while let Some(f) = s.next_element_seed(Item(self.0.at(out.len())))? {
            out.push(f.unwrap_or_default());
        }
        Ok(Some(out))
    }
    wrong_kinds!("[]favorites.Favorite", except seq);
}

/// Item is a favorite in a folder's list.
struct Item(Path);

impl Item {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl<'de> DeserializeSeed<'de> for Item {
    type Value = Option<Favorite>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Item {
    type Value = Option<Favorite>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("struct Favorite")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Self::Value, A::Error> {
        let mut f = Favorite::default();
        while let Some(k) = m.next_key::<String>()? {
            let path = self.0.key(&k);
            let field = ["name", "command", "source", "dir", "added"].into_iter().find(|n| equal_fold(&k, n));
            match field {
                Some("added") => {
                    if let Some(t) = m.next_value_seed(Text(path, "time.Time"))? {
                        f.added = time(&t).map_err(de::Error::custom)?;
                    }
                }
                Some(name) => {
                    if let Some(v) = m.next_value_seed(Text(path, "string"))? {
                        *match name {
                            "name" => &mut f.name,
                            "command" => &mut f.command,
                            "source" => &mut f.source,
                            _ => &mut f.dir,
                        } = v;
                    }
                }
                None => {
                    m.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(Some(f))
    }
    wrong_kinds!("favorites.Favorite", except map);
}

/// Text is a string field, or a time's.
struct Text(Path, &'static str);

impl<'de> DeserializeSeed<'de> for Text {
    type Value = Option<String>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Text {
    type Value = Option<String>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a string")
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Err(self.0.wrong("bool", self.1))
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Err(self.0.wrong("number", self.1))
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Err(self.0.wrong("number", self.1))
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Err(self.0.wrong("number", self.1))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
        Err(self.0.wrong("array", self.1))
    }
    fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
        Err(self.0.wrong("object", self.1))
    }
}

/// time is a time.Time read from JSON: RFC 3339, Go's zero time none, and
/// time.Parse's words for one that doesn't start with a year, the garbage
/// a hand's edit leaves.
fn time(s: &str) -> Result<Option<DateTime<Utc>>, String> {
    match parse_time(s) {
        Ok(t) if t == zero_time() => Ok(None),
        Ok(t) => Ok(Some(t)),
        Err(_) if s.len() < 4 || !s.as_bytes()[..4].iter().all(u8::is_ascii_digit) => Err(format!(
            "parsing time {} as \"2006-01-02T15:04:05Z07:00\": cannot parse {} as \"2006\"",
            quote(s),
            quote(s)
        )),
        Err(e) => Err(format!("parsing time {}: {e}", quote(s))),
    }
}
