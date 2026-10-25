//! What every crate shares: errors that wrap others the way `fmt.Errorf`'s
//! `%w` does, still found under the message; and, in `golang`, what Go's
//! standard library does that runui's words and files depend on, so the
//! Rust build says and writes what the Go build does.
//!
//! Taken from aemui's aem-core (its errors, `%q`, `encoding/json`'s bytes,
//! `time.Time` as JSON, `os`'s errors) and its path/filepath port, with
//! what runui needs besides: Go 1.27's JSON decoder read token by token,
//! `time.Duration`'s words, signals' names and `os/exec`'s errors.

mod error;
pub mod golang;

pub use error::{Wrapped, find, is, wrap};

/// Result is anyhow's: an error is any error, as Go's `error` is.
pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
pub use anyhow::{Error, anyhow, bail};
