//! Styling some characters of a string one way and the rest another.

use super::Style;

/// style_runes renders the characters of `s` at `indices` (counted in
/// characters) with `matched`, and the others with `unmatched`, a run of
/// either rendered at once.
pub fn style_runes(s: &str, indices: &[i32], matched: Style, unmatched: Style) -> String {
    let set: std::collections::HashSet<i32> = indices.iter().copied().collect();
    let runes: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut group = String::new();
    for (i, r) in runes.iter().enumerate() {
        group.push(*r);
        let i = i as i32;
        let matches = set.contains(&i);
        let next_matches = set.contains(&(i + 1));
        if matches != next_matches || i as usize == runes.len() - 1 {
            let style = if matches { matched } else { unmatched };
            out.push_str(&style.render(&group));
            group.clear();
        }
    }
    out
}
