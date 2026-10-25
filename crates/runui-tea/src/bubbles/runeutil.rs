//! Cleaning what's typed or pasted before it goes into a text field
//! (Bubbles' `internal/runeutil`): invalid characters and control
//! characters dropped, tabs and line breaks replaced.

/// Sanitizer cleans characters.
#[derive(Clone, Debug)]
pub(crate) struct Sanitizer {
    replace_newline: Vec<char>,
    replace_tab: Vec<char>,
}

impl Sanitizer {
    /// new keeps line breaks and makes tabs four spaces.
    pub(crate) fn new() -> Sanitizer {
        Sanitizer { replace_newline: vec!['\n'], replace_tab: "    ".chars().collect() }
    }

    /// replace_tabs sets what a tab becomes.
    pub(crate) fn replace_tabs(mut self, s: &str) -> Sanitizer {
        self.replace_tab = s.chars().collect();
        self
    }

    /// replace_newlines sets what a line break becomes.
    pub(crate) fn replace_newlines(mut self, s: &str) -> Sanitizer {
        self.replace_newline = s.chars().collect();
        self
    }

    /// sanitize cleans `runes`.
    pub(crate) fn sanitize(&self, runes: &[char]) -> Vec<char> {
        let mut out = Vec::with_capacity(runes.len());
        for &r in runes {
            match r {
                '\u{FFFD}' => {}
                '\r' | '\n' => out.extend_from_slice(&self.replace_newline),
                '\t' => out.extend_from_slice(&self.replace_tab),
                c if c.is_control() => {}
                c => out.push(c),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        let cases = [
            ("", ""),
            ("x", "x"),
            ("\n", "XX"),
            ("\na\n", "XXaXX"),
            ("\n\n", "XXXX"),
            ("\t", ""),
            ("hello", "hello"),
            ("hel\nlo", "helXXlo"),
            ("hel\rlo", "helXXlo"),
            ("hel\tlo", "hello"),
            ("he\n\nl\tlo", "heXXXXllo"),
            ("he\tl\n\nlo", "helXXXXlo"),
            ("hel\x1blo", "hello"),
            ("hello\u{FFFD}", "hello"),
        ];
        let s = Sanitizer::new().replace_newlines("XX").replace_tabs("");
        for (input, output) in cases {
            let runes: Vec<char> = input.chars().collect();
            assert_eq!(s.sanitize(&runes).into_iter().collect::<String>(), output, "{input:?}");
        }
    }
}
