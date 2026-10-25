//! Go's `time.Time` as JSON: RFC 3339 with nanoseconds, trailing zeros
//! cut, in local time, "Z" at UTC; and its zero time.

use chrono::{DateTime, FixedOffset, Local, NaiveDate, Timelike, Utc};
use serde::{Deserializer, Serializer};

/// zero_time is Go's zero `time.Time`: the first instant of year 1, UTC.
pub fn zero_time() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(1, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| t.and_utc()).unwrap_or_default()
}

/// format_time is t as Go writes a `time.Now()` in JSON: in local time.
pub fn format_time(t: &DateTime<Utc>) -> String {
    format_fixed(&t.with_timezone(&Local).fixed_offset())
}

/// format_fixed is t in Go's `time.RFC3339Nano`, "2006-01-02T15:04:05.999999999Z07:00":
/// the nanoseconds without their trailing zeros, none when they're all
/// zero, and "Z" for an offset of zero.
pub fn format_fixed(t: &DateTime<FixedOffset>) -> String {
    let mut s = t.format("%Y-%m-%dT%H:%M:%S").to_string();
    let nanos = t.nanosecond() % 1_000_000_000;
    if nanos != 0 {
        s.push('.');
        s.push_str(format!("{nanos:09}").trim_end_matches('0'));
    }
    let offset = t.offset().local_minus_utc();
    if offset == 0 {
        s.push('Z');
    } else {
        let sign = if offset < 0 { '-' } else { '+' };
        let offset = offset.abs();
        s.push_str(&format!("{sign}{:02}:{:02}", offset / 3600, offset % 3600 / 60));
    }
    s
}

/// parse_time reads a time as Go's `time.Time` reads one from JSON: RFC 3339.
pub fn parse_time(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc))
}

/// time is a `DateTime<Utc>` field as Go's `time.Time` is in JSON; null
/// leaves it zero.
pub mod time {
    use super::*;
    use serde::Deserialize;

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        if *t == zero_time() {
            return s.serialize_str("0001-01-01T00:00:00Z");
        }
        s.serialize_str(&format_time(t))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(zero_time()),
            Some(s) => parse_time(&s).map_err(serde::de::Error::custom),
        }
    }
}

/// opt_time is an `Option<DateTime<Utc>>` field as Go's `time.Time` is in
/// JSON, None being Go's zero time, "0001-01-01T00:00:00Z".
pub mod opt_time {
    use super::*;

    pub fn serialize<S: Serializer>(t: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        super::time::serialize(&t.unwrap_or_else(zero_time), s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let t = super::time::deserialize(d)?;
        Ok(if t == zero_time() { None } else { Some(t) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn times_are_written_as_go_writes_them() {
        // as Go 1.27's json.Marshal wrote these
        let at = |offset: i32, nanos: u32| {
            FixedOffset::east_opt(offset)
                .unwrap()
                .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
                .unwrap()
                .with_nanosecond(nanos)
                .unwrap()
        };
        assert_eq!(format_fixed(&at(3600, 120_000_000)), "2026-01-02T03:04:05.12+01:00");
        assert_eq!(format_fixed(&at(0, 0)), "2026-01-02T03:04:05Z");
        assert_eq!(format_fixed(&at(7200, 500_000_000)), "2026-01-02T03:04:05.5+02:00");
        assert_eq!(format_fixed(&at(-(3 * 3600 + 30 * 60), 287_491_000)), "2026-01-02T03:04:05.287491-03:30");
        assert_eq!(format_fixed(&at(19800, 1)), "2026-01-02T03:04:05.000000001+05:30");
        let back = parse_time("2026-01-02T03:04:05.5+02:00").unwrap();
        assert_eq!(back, Utc.with_ymd_and_hms(2026, 1, 2, 1, 4, 5).unwrap().with_nanosecond(500_000_000).unwrap());
        assert_eq!(parse_time("0001-01-01T00:00:00Z").unwrap(), zero_time());
        assert!(parse_time("2026-01-02 03:04:05").is_err());
    }
}
