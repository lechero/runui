//! Go's `strings` and `unicode` as aem-cli's matching depends on them:
//! case folded a letter at a time, one letter for one, the way Go's
//! `unicode.ToLower` and `strings.EqualFold` do it, where Rust's may make
//! one letter more.

/// equal_fold reports whether two strings are the same but for case,
/// letter by letter: Go's `strings.EqualFold`.
pub fn equal_fold(a: &str, b: &str) -> bool {
    let mut x = a.chars();
    let mut y = b.chars();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c1), Some(c2)) => {
                if c1 != c2 && simple_lower(c1) != simple_lower(c2) && simple_upper(c1) != simple_upper(c2) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// simple_lower is a letter in lower case, one for one: Go's
/// `unicode.ToLower`, where Rust's may give more than one.
pub fn simple_lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// simple_upper is a letter in upper case, one for one: Go's
/// `unicode.ToUpper`.
pub fn simple_upper(c: char) -> char {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// to_lower is s in lower case, a letter for a letter: Go's
/// `strings.ToLower`.
pub fn to_lower(s: &str) -> String {
    s.chars().map(simple_lower).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_folds_as_gos_does() {
        assert!(equal_fold("Admin", "aDMIN"));
        assert!(equal_fold("ſ", "S"));
        assert!(!equal_fold("admin", "admins"));
        assert!(!equal_fold("a", "b"));
        assert_eq!(to_lower("ÀB İ"), "àb i");
    }
}
