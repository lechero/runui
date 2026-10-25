use std::fmt;

/// Wrapped is an error said in other words that keeps the one it wraps
/// under it: Go's `fmt.Errorf("...%w...", err)`. Its message is the whole
/// message, the wrapped one's words in it where the Go code put them.
pub struct Wrapped {
    msg: String,
    source: anyhow::Error,
}

impl fmt::Display for Wrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl fmt::Debug for Wrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} (wrapping {:?})", self.msg, self.source)
    }
}

impl std::error::Error for Wrapped {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// wrap says err in other words, msg, and keeps it findable under them:
/// `fmt.Errorf("copying %s: %w", path, err)` is
/// `wrap(err, format!("copying {path}: {err}"))`.
pub fn wrap(err: impl Into<anyhow::Error>, msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Wrapped { msg: msg.into(), source: err.into() })
}

/// find is Go's `errors.As`: the first error of type T in err's chain,
/// err itself first, then what it wraps.
pub fn find<T: std::error::Error + Send + Sync + 'static>(err: &anyhow::Error) -> Option<&T> {
    err.chain().find_map(|e| e.downcast_ref::<T>())
}

/// is is Go's `errors.Is` for an error type: whether one of type T is in
/// err's chain.
pub fn is<T: std::error::Error + Send + Sync + 'static>(err: &anyhow::Error) -> bool {
    find::<T>(err).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NotFound(String);

    impl fmt::Display for NotFound {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} isn't there", self.0)
        }
    }

    impl std::error::Error for NotFound {}

    #[test]
    fn a_wrapped_error_is_found_under_its_words() {
        let inner = anyhow::Error::new(NotFound("/content".into()));
        let err = wrap(inner, "reading /content: /content isn't there");
        let err = wrap(err, "twice: it's deep");
        assert_eq!(err.to_string(), "twice: it's deep");
        assert_eq!(find::<NotFound>(&err).unwrap().0, "/content");
        assert!(!is::<std::io::Error>(&err));
        // anyhow's context keeps it findable too
        let ctx = anyhow::Error::new(NotFound("x".into())).context("with context");
        assert!(is::<NotFound>(&ctx));
    }
}
