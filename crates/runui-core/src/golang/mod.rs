//! What Go's standard library does that runui's words, files and answers
//! depend on, done the same way, so the Rust build says and writes what the
//! Go build does: `%q` quoting, `encoding/json`'s bytes, its decoder read a
//! token at a time and its errors, `time.Time` as JSON and
//! `time.Duration`'s words, `path/filepath`, `os`'s files and errors,
//! `os/exec`'s, and the names of signals.

pub mod duration;
pub mod exec;
pub mod filepath;
pub mod json;
pub mod jsontext;
pub mod os;
mod print;
pub mod quote;
pub mod signal;
pub mod strings;
pub mod times;

pub use duration::duration_string;
pub use json::{indent, marshal, marshal_indent, marshal_vec};
pub use os::{
    base, create_temp, dir, errno_words, is_not_exist, join, mkdir_all, path_error, read_dir, read_file, rename,
    write_file,
};
pub use quote::{decode_rune, go_lossy, is_print, quote, quote_bytes, quote_rune};
pub use strings::to_lower;
pub use times::{format_fixed, format_time, opt_time, parse_time, time, zero_time};
