//! How wide text is on a terminal: grapheme clusters measured the way
//! clipperhouse/displaywidth measures them (x/ansi's `GraphemeWidth`), single
//! characters the way mattn/go-runewidth does, and plain strings the way
//! rivo/uniseg does (what the text input and area measure with).

use unicode_segmentation::UnicodeSegmentation;

use super::tables::{GRAPHEME_WIDTHS, RUNE_WIDTHS, UNISEG_WIDTHS};

/// lookup is the width a table gives `c`: 1 unless a range says otherwise.
fn lookup(table: &[(u32, u32, u8)], c: char) -> i32 {
    let c = c as u32;
    match table.binary_search_by(|&(lo, hi, _)| {
        if hi < c {
            std::cmp::Ordering::Less
        } else if lo > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => table[i].2 as i32,
        Err(_) => 1,
    }
}

/// char_width is how wide `c` is when it starts a grapheme cluster, as
/// displaywidth has it.
fn char_width(c: char) -> i32 {
    if (c as u32) < 0x80 {
        return if (c as u32) <= 0x1F || c as u32 == 0x7F { 0 } else { 1 };
    }
    lookup(GRAPHEME_WIDTHS, c)
}

/// grapheme_width is how wide one grapheme cluster is: as wide as its first
/// character, two when an emoji presentation selector (VS16) follows it.
pub(crate) fn grapheme_width(g: &str) -> i32 {
    let b = g.as_bytes();
    if b.is_empty() {
        return 0;
    }
    if b.len() == 1 {
        return if b[0] <= 0x1F || b[0] == 0x7F { 0 } else { 1 };
    }
    if b[0] <= 0x1F {
        return 0;
    }
    let mut chars = g.chars();
    let first = chars.next().unwrap_or('\0');
    let mut w = char_width(first);
    if w != 2 && chars.next() == Some('\u{FE0F}') {
        w = 2;
    }
    w
}

/// first_grapheme_cluster is the grapheme cluster `s` starts with, and how
/// wide it is.
pub(crate) fn first_grapheme_cluster(s: &str) -> (&str, i32) {
    let g = s.graphemes(true).next().unwrap_or("");
    (g, grapheme_width(g))
}

/// rune_width is how many cells `c` takes, as go-runewidth's `RuneWidth`
/// counts them (East Asian ambiguous characters one wide).
pub fn rune_width(c: char) -> i32 {
    let u = c as u32;
    if u < 0x80 {
        return if u == 0 || u < 0x20 || u == 0x7F { 0 } else { 1 };
    }
    lookup(RUNE_WIDTHS, c)
}

/// uniseg_rune_width is how wide uniseg has a single character.
fn uniseg_rune_width(c: char) -> i32 {
    let u = c as u32;
    if u < 0x80 {
        return if u < 0x20 || u == 0x7F { 0 } else { 1 };
    }
    lookup(UNISEG_WIDTHS, c)
}

/// uniseg_width is how wide plain text (no escape sequences) is, as
/// uniseg's `StringWidth` measures it: grapheme by grapheme, each the width
/// of its characters, two for one shown as an emoji, the second of a flag's
/// pair of letters not counted.
pub(crate) fn uniseg_width(s: &str) -> i32 {
    let mut width = 0;
    for g in s.graphemes(true) {
        let mut chars = g.chars();
        let Some(first) = chars.next() else { continue };
        let mut w = uniseg_rune_width(first);
        let regional = ('\u{1F1E6}'..='\u{1F1FF}').contains(&first);
        let hangul_l = ('\u{1100}'..='\u{115F}').contains(&first) || ('\u{A960}'..='\u{A97C}').contains(&first);
        for c in chars {
            if c == '\u{FE0F}' {
                w = 2;
            } else if c == '\u{FE0E}' {
                w = 1;
            } else if !regional && !hangul_l {
                w += uniseg_rune_width(c);
            }
        }
        width += w;
    }
    width
}
