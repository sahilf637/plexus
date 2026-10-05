//! Plain-language reasons for links ("both mention rust, ownership"), made without an LLM from
//! what two items objectively share. They explain a link; they never decide whether it exists.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::db::{chunks, items};
use crate::error::Result;

/// Shared words named in a reason.
const MAX_WORDS: usize = 3;
/// Shared words checked against the library, most used first.
const LOOKED_UP: usize = 16;
/// A word in more than this share of items says little about any one of them.
const COMMON_SHARE: f64 = 0.3;
/// Below this many items every word is distinctive enough.
const COMMON_FROM: usize = 20;
/// Longest stretch of an item's text looked at.
const MAX_CHARS: usize = 40_000;

const STOPWORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "also", "another", "because", "been", "before", "being", "between",
    "both", "cannot", "could", "does", "doing", "done", "down", "during", "each", "even", "every", "from", "further",
    "have", "having", "here", "http", "https", "into", "just", "like", "made", "make", "many", "more", "most", "much",
    "must", "never", "only", "other", "over", "same", "should", "since", "some", "still", "such", "than", "that",
    "their", "them", "then", "there", "these", "they", "this", "those", "through", "under", "until", "very", "want",
    "were", "what", "when", "where", "which", "while", "will", "with", "within", "without", "would", "your", "www",
    "usually", "often", "around", "become", "became", "becomes", "known", "called", "including", "various", "several",
    "however", "therefore", "using", "used", "uses", "world", "addition", "newly", "below", "beneath", "among", "along",
    "either", "often", "early", "later", "first", "second", "third", "large", "small", "high", "long", "high", "well",
];

/// Lower-case words of at least four letters, counted. Numbers, link scraps and filler don't count.
fn word_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let len = word.chars().count();
        if !(4..=24).contains(&len) || word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let word = word.to_lowercase();
        if !STOPWORDS.contains(&word.as_str()) {
            *counts.entry(word).or_default() += 1;
        }
    }
    counts
}

/// The words both texts use most that few other items do: how often both use a word, times how
/// rare it is in the library. `items_with(word)` is how many items contain it, out of `total`.
pub fn shared_words(a: &str, b: &str, total: usize, mut items_with: impl FnMut(&str) -> usize) -> Vec<String> {
    let (a, b) = (word_counts(a), word_counts(b));
    let mut shared: Vec<(&String, usize)> = a.iter().filter_map(|(w, n)| Some((w, (*n).min(*b.get(w)?)))).collect();
    // Only the most used are looked up in the library.
    shared.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| y.0.len().cmp(&x.0.len())).then_with(|| x.0.cmp(y.0)));
    shared.truncate(LOOKED_UP);
    let mut scored: Vec<(&String, f64)> = shared
        .into_iter()
        .filter_map(|(w, uses)| {
            let with = items_with(w);
            if total >= COMMON_FROM && with as f64 > COMMON_SHARE * total as f64 {
                return None;
            }
            Some((w, uses as f64 * ((total as f64 + 1.0) / (with as f64 + 1.0)).ln().max(0.1)))
        })
        .collect();
    scored.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(y.0)));
    scored.into_iter().take(MAX_WORDS).map(|(w, _)| w.clone()).collect()
}

/// "example.com" from "https://www.example.com/a/b".
pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host).to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The sentence for a link, from the shared words and the items' websites.
pub fn sentence(words: &[String], same_host: Option<&str>, name_only: bool) -> String {
    let mentions = (!words.is_empty()).then(|| format!("both mention {}", words.join(", ")));
    match (same_host, mentions) {
        (Some(host), Some(m)) => format!("same site ({host}); {m}"),
        (Some(host), None) => format!("same site ({host})"),
        (None, Some(m)) => m,
        (None, None) if name_only => "similar name".to_string(),
        (None, None) => "close in meaning".to_string(),
    }
}

/// Writes a reason on each link `item_id` just made. `text` is the item's own text; the others'
/// come from their stored chunks.
pub fn explain(conn: &mut Connection, item_id: &str, text: &str, links: &[(String, f32, crate::pipeline::link::Tier)], name_only: bool) -> Result<()> {
    let total = items::count(conn)?;
    let me = items::get(conn, item_id)?;
    let my_host = me.as_ref().and_then(|i| i.source_url.as_deref()).and_then(host);
    let text: String = text.chars().take(MAX_CHARS).collect();
    let mut asked: HashMap<String, usize> = HashMap::new();
    for (other, _, _) in links {
        let theirs = chunks::text_of(conn, other, MAX_CHARS)?;
        let their_host = items::get(conn, other)?.and_then(|i| i.source_url).as_deref().and_then(host);
        let same_host = my_host.as_ref().filter(|h| Some(*h) == their_host.as_ref());
        let words = shared_words(&text, &theirs, total, |w| {
            *asked.entry(w.to_string()).or_insert_with(|| chunks::items_with_word(conn, w).unwrap_or(0))
        });
        let reason = sentence(&words, same_host.map(String::as_str), name_only);
        let (src, dst) = if item_id < other.as_str() { (item_id, other.as_str()) } else { (other.as_str(), item_id) };
        conn.execute(
            "UPDATE edges SET reason = ?3 WHERE src = ?1 AND dst = ?2 AND kind = 'similar'",
            rusqlite::params![src, dst, reason],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_distinctive_shared_words() {
        let a = "Rust ownership means every value has one owner. The borrow checker enforces ownership rules.";
        let b = "The borrow checker in Rust rejects code that breaks ownership. Rust is a systems language.";
        let words = shared_words(a, b, 5, |_| 1);
        assert!(words.contains(&"ownership".to_string()) && words.contains(&"borrow".to_string()), "{words:?}");
        assert!(words.len() <= MAX_WORDS);
    }

    #[test]
    fn leaves_out_words_most_items_have() {
        let a = "systems ownership systems systems";
        let b = "systems ownership";
        let common = |w: &str| if w == "systems" { 90 } else { 2 };
        assert_eq!(shared_words(a, b, 100, common), vec!["ownership".to_string()]);
        // In a small library everything counts.
        assert_eq!(shared_words(a, b, 10, common).len(), 2);
    }

    #[test]
    fn rare_words_beat_merely_frequent_ones() {
        let df = |w: &str| if w == "bread" { 25 } else { 1 };
        // Both use each word once; bread is in 25 of 100 items, Carlsen in one.
        assert_eq!(shared_words("bread carlsen", "bread carlsen", 100, df), vec!["carlsen".to_string(), "bread".to_string()]);
        // Using a common word a lot can still outweigh a rare one used once.
        assert_eq!(shared_words("bread bread bread carlsen", "bread bread bread carlsen", 100, |w| if w == "bread" { 12 } else { 1 })[0], "bread");
    }

    #[test]
    fn ignores_numbers_filler_and_short_words() {
        assert!(shared_words("2024 with that http www the cat", "2024 with that http www the cat", 1, |_| 0).is_empty());
    }

    #[test]
    fn explain_stores_a_reason_on_the_link() {
        use crate::db::items::{sample, upsert};
        use crate::pipeline::link::Tier;
        let (_dir, db) = crate::db::open_temp();
        db.with(|conn| {
            let mut a = sample("a", "items/a.md", "h1");
            a.source_url = Some("https://docs.example.com/one".into());
            let mut b = sample("b", "items/b.md", "h2");
            b.source_url = Some("https://example.com/two".into());
            upsert(conn, &a, "ownership borrowing lifetimes")?;
            upsert(conn, &b, "ownership borrowing")?;
            chunks::replace(conn, "b", &["ownership borrowing".into()], &[vec![0.0; 384]])?;
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, provenance, created_at) VALUES ('a', 'b', 'similar', 0.8, 'embedding', '')",
                [],
            )?;
            explain(conn, "a", "ownership borrowing lifetimes", &[("b".into(), 0.8, Tier::Strong)], false)?;
            let reason: String = conn.query_row("SELECT reason FROM edges", [], |r| r.get(0))?;
            assert_eq!(reason, "both mention borrowing, ownership");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn hosts_and_sentences() {
        assert_eq!(host("https://www.Example.com:8080/a?b#c").as_deref(), Some("example.com"));
        assert_eq!(host("not a url"), None);
        let words = vec!["rust".to_string(), "ownership".to_string()];
        assert_eq!(sentence(&words, None, false), "both mention rust, ownership");
        assert_eq!(sentence(&words, Some("doc.rust-lang.org"), false), "same site (doc.rust-lang.org); both mention rust, ownership");
        assert_eq!(sentence(&[], Some("a.com"), false), "same site (a.com)");
        assert_eq!(sentence(&[], None, true), "similar name");
        assert_eq!(sentence(&[], None, false), "close in meaning");
    }
}
