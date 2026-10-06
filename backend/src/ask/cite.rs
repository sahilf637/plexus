//! Checking an answer's sources. A source stands only if the note was one the agent was shown and
//! the quote really occurs in that note. The rest are dropped, along with their `[n]` marks, and
//! the ones that remain are numbered again from 1.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use super::prompt::Cited;
use crate::db::asks::Source;

/// A quote shorter than this proves nothing.
const MIN_QUOTE: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub text: String,
    pub sources: Vec<Source>,
    /// Sources were given and none held up (as against an answer that gave none because the notes
    /// don't cover the question).
    pub unsupported: bool,
}

/// Lowercase, one space between words, no quotation marks around it.
fn plain(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.join(" ").trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’' | '`')).to_lowercase()
}

/// `resolve` turns the model's name for a note into its id; `seen` is the notes it was shown;
/// `text_of` is what a quote may be found in.
pub fn check(text: &str, cited: &[Cited], resolve: &dyn Fn(&str) -> Option<String>, seen: &HashSet<String>, text_of: &dyn Fn(&str) -> Option<String>) -> Checked {
    let mut ordered: Vec<&Cited> = cited.iter().collect();
    ordered.sort_by_key(|c| c.n);
    ordered.dedup_by_key(|c| c.n);

    let mut kept: Vec<Source> = Vec::new();
    let mut renumbered: HashMap<i64, i64> = HashMap::new();
    for source in ordered {
        let quote = plain(&source.quote);
        let Some(id) = resolve(&source.item).filter(|id| seen.contains(id)) else { continue };
        let found = quote.chars().count() >= MIN_QUOTE && text_of(&id).is_some_and(|body| plain(&body).contains(&quote));
        if !found {
            continue;
        }
        let n = kept.len() as i64 + 1;
        renumbered.insert(source.n, n);
        kept.push(Source { n, item_id: id, quote: source.quote.trim().to_string() });
    }

    Checked { text: renumber(text, &renumbered), unsupported: !cited.is_empty() && kept.is_empty(), sources: kept }
}

/// Rewrites `[2]` and `[1, 3]` with the new numbers; marks for dropped sources disappear.
fn renumber(text: &str, map: &HashMap<i64, i64>) -> String {
    static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]?\[(\d{1,2}(?:\s*,\s*\d{1,2})*)\]").unwrap());
    MARK.replace_all(text, |caps: &regex::Captures| {
        let mut numbers: Vec<i64> = caps[1].split(',').filter_map(|n| n.trim().parse::<i64>().ok()).filter_map(|n| map.get(&n).copied()).collect();
        numbers.sort_unstable();
        numbers.dedup();
        if numbers.is_empty() {
            return String::new();
        }
        let joined: Vec<String> = numbers.iter().map(i64::to_string).collect();
        let space = if caps[0].starts_with([' ', '\t']) { " " } else { "" };
        format!("{space}[{}]", joined.join(", "))
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cited(n: i64, item: &str, quote: &str) -> Cited {
        Cited { n, item: item.into(), quote: quote.into() }
    }

    fn run(text: &str, cited: &[Cited]) -> Checked {
        let seen: HashSet<String> = ["i1", "i2"].map(String::from).into();
        let resolve = |name: &str| match name {
            "n1" => Some("i1".to_string()),
            "n2" => Some("i2".to_string()),
            "n3" => Some("i3".to_string()),
            _ => None,
        };
        let text_of = |id: &str| match id {
            "i1" => Some("Each value has   one owner.\nBorrowing lends it out.".to_string()),
            "i2" => Some("The borrow checker enforces the rules.".to_string()),
            "i3" => Some("A note that was never shown, with a real quote in it.".to_string()),
            _ => None,
        };
        check(text, cited, &resolve, &seen, &text_of)
    }

    #[test]
    fn a_quote_found_in_the_note_stands_whatever_the_spacing_and_case() {
        let out = run("One owner [1].", &[cited(1, "n1", "“each VALUE has one owner”")]);
        assert_eq!(out.text, "One owner [1].");
        assert_eq!(out.sources, vec![Source { n: 1, item_id: "i1".into(), quote: "“each VALUE has one owner”".into() }]);
        assert!(!out.unsupported);
    }

    #[test]
    fn a_made_up_quote_a_note_never_shown_and_an_unknown_name_are_all_dropped() {
        let out = run(
            "A [1]. B [2]. C [3]. D [4].",
            &[
                cited(1, "n1", "the owner is always a person"),
                cited(2, "n3", "with a real quote in it"),
                cited(3, "n9", "anything at all"),
                cited(4, "n2", "borrow checker enforces"),
            ],
        );
        assert_eq!(out.text, "A. B. C. D [1].");
        assert_eq!(out.sources.len(), 1);
        assert_eq!(out.sources[0].item_id, "i2");
        assert!(!out.unsupported);
    }

    #[test]
    fn what_remains_is_numbered_again_from_one() {
        let out = run(
            "First [2], then both [1, 3].",
            &[cited(1, "n1", "wrong words entirely"), cited(2, "n1", "lends it out"), cited(3, "n2", "enforces the rules")],
        );
        assert_eq!(out.text, "First [1], then both [2].");
        assert_eq!(out.sources.iter().map(|s| s.n).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn an_answer_whose_sources_all_fail_is_unsupported_but_one_with_none_is_not() {
        let out = run("Because [1].", &[cited(1, "n1", "something invented here")]);
        assert!(out.unsupported && out.sources.is_empty());
        assert_eq!(out.text, "Because.");
        let none = run("Your notes don't cover this.", &[]);
        assert!(!none.unsupported);
        assert_eq!(none.text, "Your notes don't cover this.");
    }

    #[test]
    fn a_very_short_quote_proves_nothing() {
        let out = run("Yes [1].", &[cited(1, "n1", "owner")]);
        assert!(out.sources.is_empty());
        let out = run("In [2024] it was fine.", &[]);
        assert_eq!(out.text, "In [2024] it was fine.");
    }
}
