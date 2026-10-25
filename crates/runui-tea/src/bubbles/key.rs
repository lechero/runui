//! Key bindings: the keys an action answers to, and how help shows them
//! (Bubbles' `key`).
//!
//! ```
//! use runui_tea::bubbles::key::{self, Binding};
//! use runui_tea::tea::KeyPressMsg;
//!
//! let next = Binding::new().with_keys(&["tab", "l"]).with_help("tab/l", "next panel");
//! assert!(key::matches(&KeyPressMsg::parse("tab"), &[&next]));
//! assert!(!key::matches(&KeyPressMsg::parse("h"), &[&next]));
//! ```

use std::fmt;

/// Help is how a binding shows in help: its keys and what it does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Help {
    /// key is the keys, as help shows them ("tab/l").
    pub key: String,
    /// desc is what the keys do ("next panel").
    pub desc: String,
}

/// Binding is the keys one action answers to, with its help.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Binding {
    keys: Option<Vec<String>>,
    help: Help,
    disabled: bool,
}

impl Binding {
    /// new is a binding with no keys: Go's `key.NewBinding()`, its options
    /// the `with_` methods.
    pub fn new() -> Binding {
        Binding::default()
    }

    /// with_keys sets the keys (Go's `key.WithKeys`).
    pub fn with_keys(mut self, keys: &[&str]) -> Binding {
        self.set_keys(keys);
        self
    }

    /// with_help sets the help (Go's `key.WithHelp`).
    pub fn with_help(mut self, key: &str, desc: &str) -> Binding {
        self.set_help(key, desc);
        self
    }

    /// with_disabled starts the binding disabled (Go's `key.WithDisabled`).
    pub fn with_disabled(mut self) -> Binding {
        self.disabled = true;
        self
    }

    /// set_keys sets the keys.
    pub fn set_keys(&mut self, keys: &[&str]) {
        self.keys = if keys.is_empty() { None } else { Some(keys.iter().map(|k| k.to_string()).collect()) };
    }

    /// keys is the keys.
    pub fn keys(&self) -> &[String] {
        self.keys.as_deref().unwrap_or(&[])
    }

    /// set_help sets the help.
    pub fn set_help(&mut self, key: &str, desc: &str) {
        self.help = Help { key: key.to_string(), desc: desc.to_string() };
    }

    /// help is the help.
    pub fn help(&self) -> &Help {
        &self.help
    }

    /// enabled says whether the binding answers: it isn't disabled and
    /// has keys.
    pub fn enabled(&self) -> bool {
        !self.disabled && self.keys.is_some()
    }

    /// set_enabled enables or disables the binding.
    pub fn set_enabled(&mut self, v: bool) {
        self.disabled = !v;
    }

    /// unbind takes the keys and help away.
    pub fn unbind(&mut self) {
        self.keys = None;
        self.help = Help::default();
    }
}

/// matches says whether `k` (a key press, or anything that writes as a
/// key's name) is one of the enabled bindings' keys.
pub fn matches<K: fmt::Display + ?Sized>(k: &K, bindings: &[&Binding]) -> bool {
    let s = k.to_string();
    bindings.iter().any(|b| b.enabled() && b.keys().contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tea::KeyPressMsg;

    #[test]
    fn binding_enabled() {
        let mut b = Binding::new().with_keys(&["k", "up"]).with_help("↑/k", "move up");
        assert!(b.enabled());
        b.set_enabled(false);
        assert!(!b.enabled());
        b.set_enabled(true);
        b.unbind();
        assert!(!b.enabled());
    }

    #[test]
    fn dotted_keys() {
        let b = Binding::new().with_keys(&["."]);
        assert!(matches(&KeyPressMsg::char('.'), &[&b]));
        assert!(!matches(&KeyPressMsg::char(','), &[&b]));
    }

    #[test]
    fn disabled_doesnt_match() {
        let b = Binding::new().with_keys(&["q"]).with_disabled();
        assert!(!matches(&KeyPressMsg::char('q'), &[&b]));
        assert!(!matches(&KeyPressMsg::char('q'), &[&Binding::new()]));
    }
}
