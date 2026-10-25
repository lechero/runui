//! Fuzzy matching as sahilm/fuzzy does it (what the list filters with):
//! every character of the pattern found in order, scored for matching the
//! first character, after a separator, at a camel-case hump and next to
//! the last match, less for what's skipped.

const FIRST_CHAR_MATCH_BONUS: i32 = 10;
const MATCH_FOLLOWING_SEPARATOR_BONUS: i32 = 20;
const CAMEL_CASE_MATCH_BONUS: i32 = 20;
const ADJACENT_MATCH_BONUS: i32 = 5;
const UNMATCHED_LEADING_CHAR_PENALTY: i32 = -5;
const MAX_UNMATCHED_LEADING_CHAR_PENALTY: i32 = -15;

const SEPARATORS: [char; 6] = ['/', '-', '_', ' ', '.', '\\'];

/// Match is a string the pattern matched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Match {
    /// str is the string matched.
    pub str: String,
    /// index is where it was in the list searched.
    pub index: i32,
    /// matched_indexes is where each pattern character matched, as byte
    /// offsets into the string.
    pub matched_indexes: Vec<i32>,
    /// score is how good the match is: higher is better.
    pub score: i32,
}

fn simple_fold(r: char) -> char {
    // the next character in r's case-folding orbit, as Go's unicode.SimpleFold
    let lower = r.to_lowercase().next().unwrap_or(r);
    let upper = r.to_uppercase().next().unwrap_or(r);
    let mut orbit = vec![r];
    for c in [lower, upper] {
        if !orbit.contains(&c) {
            orbit.push(c);
        }
    }
    orbit.sort();
    let i = orbit.iter().position(|&c| c == r).unwrap_or(0);
    orbit[(i + 1) % orbit.len()]
}

fn equal_fold(tr: char, sr: char) -> bool {
    if tr == sr {
        return true;
    }
    let (mut tr, mut sr) = (tr, sr);
    if tr < sr {
        std::mem::swap(&mut tr, &mut sr);
    }
    if (tr as u32) < 0x80 {
        return sr.is_ascii_uppercase() && tr as u32 == sr as u32 + ('a' as u32 - 'A' as u32);
    }
    let mut r = simple_fold(sr);
    while r != sr && r < tr {
        r = simple_fold(r);
    }
    r == tr
}

fn is_separator(c: char) -> bool {
    SEPARATORS.contains(&c)
}

fn adjacent_char_bonus(i: i32, last_match: i32, current_bonus: i32) -> i32 {
    if last_match == i {
        return current_bonus * 2 + ADJACENT_MATCH_BONUS;
    }
    0
}

/// find matches `pattern` against every string in `data`, best first.
pub fn find(pattern: &str, data: &[String]) -> Vec<Match> {
    let mut matches = find_no_sort(pattern, data);
    // sort.Stable on score, descending
    matches.sort_by_key(|m| std::cmp::Reverse(m.score));
    matches
}

/// find_no_sort matches `pattern` against every string in `data`, in order.
pub fn find_no_sort(pattern: &str, data: &[String]) -> Vec<Match> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let runes: Vec<char> = pattern.chars().collect();
    let mut matches = Vec::new();
    for (i, match_str) in data.iter().enumerate() {
        let mut m = Match { str: match_str.clone(), index: i as i32, ..Default::default() };
        let clean: &str = match match_str.find('\0') {
            Some(n) => &match_str[..n],
            None => match_str,
        };
        let mut score;
        let mut pattern_index = 0usize;
        let mut best_score = -1;
        let mut matched_index = -1;
        let mut curr_adjacent_match_bonus = 0;
        let mut last: char = '\0';
        let mut last_index = 0;
        let bytes = clean.as_bytes();
        let decode = |at: usize| -> (char, usize) {
            match clean[at..].chars().next() {
                Some(c) => (c, c.len_utf8()),
                None => ('\0', 0),
            }
        };
        let (mut nextc, mut next_size) = if clean.is_empty() { ('\0', 0) } else { decode(0) };
        let mut j = 0usize;
        while j < bytes.len() {
            let (candidate, candidate_size) = (nextc, next_size);
            if pattern_index < runes.len() && equal_fold(candidate, runes[pattern_index]) {
                score = 0;
                if j == 0 {
                    score += FIRST_CHAR_MATCH_BONUS;
                }
                if last.is_lowercase() && candidate.is_uppercase() {
                    score += CAMEL_CASE_MATCH_BONUS;
                }
                if j != 0 && is_separator(last) {
                    score += MATCH_FOLLOWING_SEPARATOR_BONUS;
                }
                if let Some(&last_match) = m.matched_indexes.last() {
                    let bonus = adjacent_char_bonus(last_index, last_match, curr_adjacent_match_bonus);
                    score += bonus;
                    // adjacent matches are incremental and keep increasing
                    // based on previous adjacent matches thus we need to
                    // maintain the current match bonus
                    curr_adjacent_match_bonus += bonus;
                }
                if score > best_score {
                    best_score = score;
                    matched_index = j as i32;
                }
            }
            let nextp = if pattern_index + 1 < runes.len() { runes[pattern_index + 1] } else { '\0' };
            if j + candidate_size < bytes.len() {
                (nextc, next_size) = decode(j + candidate_size);
            } else {
                (nextc, next_size) = ('\0', 0);
            }
            // We apply the best score when we have the next match coming
            // up or when the search string has ended. Tracking when the
            // next match is coming up allows us to exhaustively find the
            // best match and not necessarily the first match. For example
            // given the pattern "tk" and search string "The Black Knight",
            // exhaustively matching allows us to match the second k thus
            // giving this string a higher score.
            if (equal_fold(nextp, nextc) || nextc == '\0') && matched_index > -1 {
                if m.matched_indexes.is_empty() {
                    let penalty = matched_index * UNMATCHED_LEADING_CHAR_PENALTY;
                    best_score += penalty.max(MAX_UNMATCHED_LEADING_CHAR_PENALTY);
                }
                m.score += best_score;
                m.matched_indexes.push(matched_index);
                best_score = -1;
                pattern_index += 1;
            }
            last_index = j as i32;
            last = candidate;
            j += candidate_size.max(1);
        }
        // apply penalty for each unmatched character
        let penalty = m.matched_indexes.len() as i32 - clean.len() as i32;
        m.score += penalty;
        if m.matched_indexes.len() == runes.len() {
            matches.push(m);
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_in_order_best_first() {
        // as sahilm/fuzzy scores them
        let data = strs(&[
            "/apps/site/components/teaser/teaser.html",
            "/apps/site-packages/application/install/site.ui.config-1.0.zip",
            "/conf/site/settings",
        ]);
        let m = find("stc", &data);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].index, m[0].score, m[0].matched_indexes.clone()), (0, -12, vec![6, 8, 11]));
        assert_eq!((m[1].index, m[1].score, m[1].matched_indexes.clone()), (1, -34, vec![6, 8, 48]));
    }

    #[test]
    fn exhaustive_match() {
        let m = find("tk", &strs(&["The Black Knight"]));
        assert_eq!(m[0].matched_indexes, vec![0, 10]);
    }

    #[test]
    fn case_insensitive_and_unicode() {
        let m = find("ÉT", &strs(&["été"]));
        assert_eq!(m.len(), 1);
        assert!(find("zz", &strs(&["abc"])).is_empty());
        assert!(find("", &strs(&["abc"])).is_empty());
    }
}
