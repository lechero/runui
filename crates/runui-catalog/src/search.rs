use crate::{Entry, Kind, Origin};

#[derive(Clone, Debug)]
pub struct Query {
    terms: Vec<Term>,
    pub limit: usize,
    pub offset: usize,
    pub kinds: Vec<Kind>,
    pub registry_id: Option<String>,
}
#[derive(Clone, Debug)]
struct Term {
    facet: Option<String>,
    value: String,
}

impl Query {
    /// Recognized facets are explicit; unknown colons remain ordinary text.
    /// Quoting a token or escaping its colon forces a literal search.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut terms = Vec::new();
        let mut word = String::new();
        let mut quote = None;
        let mut literal = false;
        let mut escaped = false;
        let push = |word: &mut String, literal: bool, terms: &mut Vec<Term>| -> Result<(), String> {
            if word.is_empty() {
                return Ok(());
            }
            let mut facet = None;
            let mut value = std::mem::take(word).to_lowercase();
            if !literal
                && let Some((key, tail)) = value.split_once(':')
                && ["project", "source", "dir", "kind", "state"].contains(&key)
            {
                if tail.is_empty() {
                    return Err(format!("{key}: needs a value"));
                }
                facet = Some(key.into());
                value = tail.into();
            }
            terms.push(Term { facet, value });
            Ok(())
        };
        for ch in text.chars() {
            if escaped {
                if ch == ':' && !word.contains(':') {
                    literal = true;
                }
                word.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if let Some(active) = quote {
                if ch == active {
                    quote = None;
                } else {
                    word.push(ch);
                }
                continue;
            }
            if ch == '\'' || ch == '"' {
                quote = Some(ch);
                literal |= word.is_empty();
                continue;
            }
            if ch.is_whitespace() {
                push(&mut word, literal, &mut terms)?;
                literal = false;
            } else {
                word.push(ch);
            }
        }
        if escaped {
            word.push('\\');
        }
        if quote.is_some() {
            return Err("close the quoted search token".into());
        }
        push(&mut word, literal, &mut terms)?;
        Ok(Self { terms, limit: 200, offset: 0, kinds: Vec::new(), registry_id: None })
    }
}

#[derive(Clone, Debug)]
pub struct Hit<'a> {
    pub entry: &'a Entry,
    pub score: i64,
}

pub(crate) fn search<'a>(entries: &'a [Entry], query: &Query) -> Vec<Hit<'a>> {
    let mut hits = Vec::new();
    for entry in entries {
        if query.registry_id.as_ref().is_some_and(|id| entry.registry_id.as_ref() != Some(id)) {
            continue;
        }
        if !query.kinds.is_empty() && !query.kinds.contains(&entry.kind) {
            continue;
        }
        let mut score = 0;
        let mut matched = true;
        for term in &query.terms {
            if let Some(facet) = &term.facet {
                if !facet_matches(entry, facet, &term.value) {
                    matched = false;
                    break;
                }
            } else if let Some(points) = text_match(entry, &term.value) {
                score += points;
            } else {
                matched = false;
                break;
            }
        }
        if !matched {
            continue;
        }
        if query.terms.is_empty() {
            score = if entry.favorite {
                3000
            } else if entry.launches > 0 || entry.kind == Kind::Recent {
                2000
            } else if entry.kind == Kind::Run {
                0
            } else {
                1000
            };
        }
        hits.push(Hit { entry, score });
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.entry.favorite.cmp(&a.entry.favorite))
            .then(b.entry.last_used.cmp(&a.entry.last_used))
            .then(b.entry.launches.cmp(&a.entry.launches))
            .then(a.entry.proximity.cmp(&b.entry.proximity))
            .then(a.entry.directory.cmp(&b.entry.directory))
            .then(a.entry.name.cmp(&b.entry.name))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    hits.into_iter().skip(query.offset).take(query.limit.min(1000)).collect()
}

fn facet_matches(entry: &Entry, facet: &str, value: &str) -> bool {
    match facet {
        "project" => {
            entry.project.to_lowercase().contains(value) || entry.project_directory.to_lowercase().contains(value)
        }
        "source" => entry.source.eq_ignore_ascii_case(value),
        "dir" => entry.relative_directory.to_lowercase().contains(value),
        "state" => entry.state.as_deref() == Some(value),
        "kind" => match value {
            "command" => entry.kind == Kind::Command,
            "favorite" => entry.favorite,
            "run" => entry.kind == Kind::Run,
            "recent" => entry.kind == Kind::Recent,
            "generated" => entry.origin == Origin::Generated,
            "declared" => entry.origin == Origin::Declared,
            _ => false,
        },
        _ => false,
    }
}

fn text_match(entry: &Entry, term: &str) -> Option<i64> {
    let name = entry.name.to_lowercase();
    let mut best = quality(&name, term).map(|quality| quality + 600);
    for alias in &entry.favorite_names {
        best = best.max(quality(&alias.to_lowercase(), term).map(|quality| quality + 600));
    }
    for (text, weight) in [
        (entry.relative_directory.as_str(), 350),
        (entry.project.as_str(), 350),
        (entry.checkout.as_str(), 300),
        (entry.branch.as_deref().unwrap_or_default(), 250),
        (entry.command.as_str(), 150),
        (entry.description.as_str(), 0),
    ] {
        best = best.max(quality(&text.to_lowercase(), term).map(|quality| quality + weight));
    }
    best
}

fn quality(text: &str, term: &str) -> Option<i64> {
    if text == term {
        return Some(600);
    }
    if text.starts_with(term) {
        return Some(450);
    }
    if text.contains(term) {
        return Some(350);
    }
    let mut haystack = text.chars();
    let mut gaps = 0;
    for needle in term.chars() {
        loop {
            match haystack.next() {
                Some(ch) if ch == needle => break,
                Some(_) => gaps += 1,
                None => return None,
            }
        }
    }
    Some((200 - gaps).max(0))
}
