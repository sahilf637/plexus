//! Names for topics that need no AI: a topic is called after what is most particular to it, the
//! concepts its notes share, else the words they use that other topics don't.

use std::collections::{HashMap, HashSet};

/// What a topic's name is made from, for one item.
#[derive(Clone, Debug, Default)]
pub struct ItemWords {
    /// Terms in the title.
    pub title: HashSet<String>,
    /// Terms in the body (the start of it).
    pub body: HashSet<String>,
    /// The word as written for each term, so a name shows "Davis" and not its stem "davi".
    pub surface: HashMap<String, String>,
    /// The AI's concepts for the item.
    pub concepts: Vec<String>,
}

impl ItemWords {
    pub fn new(title: &str, body: &str, concepts: Vec<String>) -> Self {
        let mut surface = HashMap::new();
        let title = terms_into(title, &mut surface);
        let body = terms_into(body, &mut surface);
        Self { title, body, surface, concepts }
    }
}

/// A name needs a term or concept present in at least this share of the topic's items.
const MIN_COVERAGE: f64 = 0.25;
const MAX_NAME_CHARS: usize = 40;

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was", "one", "our", "out", "has", "his",
    "how", "its", "may", "new", "now", "old", "see", "two", "who", "did", "get", "him", "let", "put", "say", "she", "too", "use",
    "that", "with", "have", "this", "will", "your", "from", "they", "know", "want", "been", "good", "much", "some", "time", "very",
    "when", "come", "here", "just", "like", "long", "make", "many", "more", "only", "over", "such", "take", "than", "them", "well",
    "were", "what", "which", "their", "there", "these", "those", "about", "after", "again", "also", "because", "before", "being",
    "could", "every", "first", "into", "other", "should", "still", "then", "through", "under", "using", "where", "while", "would",
    "note", "notes", "http", "https", "www", "com", "html", "pdf", "image", "pasted", "untitled",
    "called", "made", "gives", "give", "given", "part", "parts", "cause", "causes", "caused", "used", "uses", "since", "several",
    "often", "during", "between", "around", "within", "however", "known", "form", "forms", "form", "second", "third", "high",
    "low", "large", "small", "different", "same", "most", "each", "both", "another", "make", "makes", "making", "called",
    "become", "becomes", "include", "includes", "including", "example", "among", "along", "must", "might", "does", "done",
    "per", "yet", "get", "gets", "back", "way", "ways", "years", "year", "hours", "hour", "day", "days", "set", "sets",
];

/// The terms of a text: lowercase words of three or more letters, without common words and with
/// plural endings folded away, so "notes" and "note" count as one. Also notes the word as
/// written (lowercase) for each term.
fn terms_into(text: &str, surface: &mut HashMap<String, String>) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| {
            let key = stem(&w);
            surface.entry(key.clone()).or_insert(w);
            key
        })
        .collect()
}

fn stem(word: &str) -> String {
    if word.chars().count() > 4 && word.ends_with('s') && !word.ends_with("ss") && !word.ends_with("us") {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

/// One topic to name: its id, where it sits (siblings can't share a name), and its items.
pub struct ToName<'a> {
    pub id: &'a str,
    pub parent: Option<&'a str>,
    pub level: i64,
    pub members: &'a [String],
    /// Its sub-topics' ids, biggest first.
    pub children: Vec<&'a str>,
}

/// Names for these topics, keyed by id. `words` has what each item says; the topics are
/// compared with the others at the same level, so a term common to all of them names none.
pub fn auto_names(topics: &[ToName], words: &HashMap<String, ItemWords>) -> HashMap<String, String> {
    // How much of each topic uses each term (title use counts fully, body use half), and
    // concepts likewise.
    struct Profile {
        terms: HashMap<String, f64>,
        concepts: HashMap<String, f64>,
    }
    let profile = |members: &[String]| -> Profile {
        let mut terms: HashMap<String, f64> = HashMap::new();
        let mut concepts: HashMap<String, f64> = HashMap::new();
        for id in members {
            let Some(w) = words.get(id) else { continue };
            for t in &w.title {
                *terms.entry(t.clone()).or_insert(0.0) += 1.0;
            }
            for t in w.body.difference(&w.title) {
                *terms.entry(t.clone()).or_insert(0.0) += 0.5;
            }
            for c in &w.concepts {
                *concepts.entry(c.clone()).or_insert(0.0) += 1.0;
            }
        }
        let size = members.len().max(1) as f64;
        terms.values_mut().for_each(|v| *v /= size);
        concepts.values_mut().for_each(|v| *v /= size);
        Profile { terms, concepts }
    };
    let profiles: Vec<Profile> = topics.iter().map(|t| profile(t.members)).collect();
    // The word each term is usually written as, over all items.
    let surface: HashMap<String, String> = {
        let mut votes: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
        for w in words.values() {
            for (term, word) in &w.surface {
                *votes.entry(term.as_str()).or_default().entry(word.as_str()).or_insert(0) += 1;
            }
        }
        votes
            .into_iter()
            .filter_map(|(term, forms)| {
                let best = forms.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))?;
                Some((term.to_string(), best.0.to_string()))
            })
            .collect()
    };

    // In how many topics of the same level each term is prominent.
    let mut by_level: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, t) in topics.iter().enumerate() {
        by_level.entry(t.level).or_default().push(i);
    }
    let mut spread: HashMap<(i64, &str), usize> = HashMap::new();
    for (i, t) in topics.iter().enumerate() {
        for (term, share) in &profiles[i].terms {
            if *share >= MIN_COVERAGE / 2.0 {
                *spread.entry((t.level, term.as_str())).or_insert(0) += 1;
            }
        }
    }

    // What each topic could be called, best first, with how many of its items use it.
    struct Candidate {
        score: f64,
        coverage: f64,
        name: String,
    }
    let mut candidates: Vec<Vec<Candidate>> = Vec::new();
    for (i, t) in topics.iter().enumerate() {
        let peers = by_level[&t.level].len() as f64;
        let mut found: Vec<Candidate> = Vec::new();
        // A concept the topic's items share is the best name there is.
        for (concept, share) in &profiles[i].concepts {
            if *share >= MIN_COVERAGE {
                found.push(Candidate { score: 10.0 + share, coverage: *share, name: concept.clone() });
            }
        }
        for (term, share) in &profiles[i].terms {
            if *share < MIN_COVERAGE {
                continue;
            }
            let others = spread.get(&(t.level, term.as_str())).copied().unwrap_or(1) as f64;
            let rarity = ((peers + 1.0) / (others + 0.5)).ln() + 1.0;
            found.push(Candidate { score: share * rarity, coverage: *share, name: show(term, &surface) });
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
        candidates.push(found);
    }

    // Top down, so a sub-topic can avoid the words its parents already use.
    let index: HashMap<&str, usize> = topics.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut order: Vec<usize> = (0..topics.len()).collect();
    order.sort_by_key(|&i| (topics[i].level, topics[i].id));
    let mut picked: Vec<Vec<String>> = vec![Vec::new(); topics.len()];
    for &i in &order {
        let t = &topics[i];
        let mut used: HashSet<String> = HashSet::new();
        let mut at = t.parent;
        while let Some(parent) = at.and_then(|p| index.get(p)) {
            used.extend(picked[*parent].iter().take(2).map(|p| p.to_lowercase()));
            at = topics[*parent].parent;
        }
        let clashes = |name: &str, chosen: &[String]| {
            let low = name.to_lowercase();
            used.iter().chain(chosen.iter().map(|c| c.to_lowercase()).collect::<Vec<_>>().iter()).any(|u| u.contains(&low) || low.contains(u.as_str()))
        };
        let strong = candidates[i].first().is_some_and(|c| c.coverage > 0.6);
        let mut chosen: Vec<String> = Vec::new();
        // A topic that is a mix of sub-topics is called after them ("Coffee & Bread").
        if !strong && t.children.len() >= 2 {
            for child in &t.children {
                let Some(first) = index.get(child).and_then(|&c| candidates[c].first()) else { continue };
                if !clashes(&first.name, &chosen) {
                    chosen.push(first.name.clone());
                }
                if chosen.len() == 3 {
                    break;
                }
            }
        }
        if chosen.len() < 2 {
            for c in &candidates[i] {
                if !clashes(&c.name, &chosen) {
                    chosen.push(c.name.clone());
                }
                if chosen.len() == 4 {
                    break;
                }
            }
        }
        // Everything it could be called is already in a parent's name: it may repeat one.
        if chosen.is_empty() {
            chosen = candidates[i].iter().take(2).map(|c| c.name.clone()).collect();
        }
        picked[i] = chosen;
    }

    // Two terms per name; siblings with the same name take another term, then a number.
    let mut used: HashMap<Option<&str>, HashSet<String>> = HashMap::new();
    let mut names = HashMap::new();
    for &i in &order {
        let t = &topics[i];
        let picks = &picked[i];
        let taken = used.entry(t.parent).or_default();
        let name = (2..=picks.len().max(2))
            .map(|n| join(&picks[..n.min(picks.len())]))
            .chain(std::iter::once(join(&picks[..picks.len().min(1)])))
            .find(|name| !name.is_empty() && !taken.contains(name))
            .unwrap_or_else(|| numbered(taken, if picks.is_empty() { "Mixed".to_string() } else { join(&picks[..1]) }));
        taken.insert(name.clone());
        names.insert(t.id.to_string(), name);
    }
    names
}

/// The word as most often written, for a term.
fn show(term: &str, surface: &HashMap<String, String>) -> String {
    surface.get(term).cloned().unwrap_or_else(|| term.to_string())
}

/// `base`, or `base 2`, `base 3`… if a sibling already has it.
fn numbered(taken: &HashSet<String>, base: String) -> String {
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base} {n}")).find(|name| !taken.contains(name)).unwrap_or(base)
}

/// "rust & ownership" → "Rust & Ownership", kept short.
fn join(terms: &[String]) -> String {
    let mut name = String::new();
    for term in terms {
        let next = if name.is_empty() { title_case(term) } else { format!("{name} & {}", title_case(term)) };
        if next.chars().count() > MAX_NAME_CHARS && !name.is_empty() {
            break;
        }
        name = next;
    }
    name
}

fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(title: &str, body: &str, concepts: &[&str]) -> ItemWords {
        ItemWords::new(title, body, concepts.iter().map(|c| c.to_string()).collect())
    }

    #[test]
    fn terms_are_lowercase_stemmed_and_skip_common_words() {
        let t = terms_into("The Rust borrow checkers, and 2024 notes", &mut HashMap::new());
        assert!(t.contains("rust") && t.contains("borrow") && t.contains("checker"));
        assert!(!t.contains("the") && !t.contains("and") && !t.contains("2024") && !t.contains("note"));
    }

    #[test]
    fn a_topic_is_named_after_what_is_particular_to_it() {
        let mut w = HashMap::new();
        let rust = ["Rust ownership rules", "Rust borrow checker", "Rust lifetimes explained", "Cargo workspaces in Rust"];
        let bread = ["Sourdough starter feeding", "Baking sourdough bread", "Bread crust steam", "Sourdough hydration"];
        for (i, t) in rust.iter().enumerate() {
            w.insert(format!("r{i}"), words(t, "systems programming code compiler memory", &[]));
        }
        for (i, t) in bread.iter().enumerate() {
            w.insert(format!("b{i}"), words(t, "flour water oven dough kitchen", &[]));
        }
        let r: Vec<String> = (0..4).map(|i| format!("r{i}")).collect();
        let b: Vec<String> = (0..4).map(|i| format!("b{i}")).collect();
        let topics = [
            ToName { id: "t1", parent: None, level: 0, members: &r, children: vec![] },
            ToName { id: "t2", parent: None, level: 0, members: &b, children: vec![] },
        ];
        let names = auto_names(&topics, &w);
        assert!(names["t1"].to_lowercase().contains("rust"), "got {}", names["t1"]);
        assert!(names["t2"].to_lowercase().contains("sourdough") || names["t2"].to_lowercase().contains("bread"), "got {}", names["t2"]);
        assert_ne!(names["t1"], names["t2"]);
    }

    #[test]
    fn a_shared_concept_wins_and_siblings_get_distinct_names() {
        let mut w = HashMap::new();
        for i in 0..4 {
            w.insert(format!("a{i}"), words("alpha", "", &["Memory safety"]));
            w.insert(format!("b{i}"), words("alpha", "", &["Memory safety"]));
        }
        let a: Vec<String> = (0..4).map(|i| format!("a{i}")).collect();
        let b: Vec<String> = (0..4).map(|i| format!("b{i}")).collect();
        let topics = [
            ToName { id: "x", parent: Some("p"), level: 1, members: &a, children: vec![] },
            ToName { id: "y", parent: Some("p"), level: 1, members: &b, children: vec![] },
        ];
        let names = auto_names(&topics, &w);
        assert!(names["x"].to_lowercase().starts_with("memory safety"), "got {}", names["x"]);
        assert_ne!(names["x"], names["y"], "siblings can't share a name");
    }

    #[test]
    fn a_topic_with_nothing_to_go_on_is_still_named() {
        let topics = [ToName { id: "x", parent: None, level: 0, members: &[], children: vec![] }];
        assert_eq!(auto_names(&topics, &HashMap::new())["x"], "Mixed");
    }
}
