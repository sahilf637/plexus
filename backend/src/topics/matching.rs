//! Carrying topics across a rebuild: a new group that holds mostly the same items as an old topic
//! *is* that topic, and keeps its id (and so its name and place on the map).

use std::collections::{HashMap, HashSet};

/// How much two member lists must overlap (shared / either) to be the same topic.
pub const MIN_OVERLAP: f64 = 0.5;

/// A topic as the matcher sees it: where it sits in the tree and who is in it.
pub struct Candidate<'a, K> {
    pub key: K,
    pub level: i64,
    pub members: &'a HashSet<String>,
}

pub fn overlap(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.iter().filter(|id| b.contains(*id)).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// Pairs each new group with the old topic it continues: best overlaps first, each used once, at
/// the same level, and only when they share at least [`MIN_OVERLAP`]. Returns new key → old id.
pub fn carry_over<K: Copy + Eq + std::hash::Hash + Ord>(old: &[Candidate<String>], new: &[Candidate<K>]) -> HashMap<K, String> {
    let mut scored: Vec<(f64, &String, K)> = Vec::new();
    for o in old {
        for n in new.iter().filter(|n| n.level == o.level) {
            let score = overlap(o.members, n.members);
            if score >= MIN_OVERLAP {
                scored.push((score, &o.key, n.key));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)).then_with(|| a.2.cmp(&b.2)));
    let mut used_old: HashSet<&String> = HashSet::new();
    let mut matched: HashMap<K, String> = HashMap::new();
    for (_, old_id, key) in scored {
        if used_old.contains(old_id) || matched.contains_key(&key) {
            continue;
        }
        used_old.insert(old_id);
        matched.insert(key, old_id.clone());
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_group_that_mostly_kept_its_members_keeps_the_topic() {
        let (a, b) = (set(&["1", "2", "3", "4"]), set(&["5", "6", "7"]));
        let old = [
            Candidate { key: "rust".to_string(), level: 0, members: &a },
            Candidate { key: "bread".to_string(), level: 0, members: &b },
        ];
        // Rust gained two items and lost none; bread lost most of its members to a new group.
        let (grown, shrunk, fresh) = (set(&["1", "2", "3", "4", "8", "9"]), set(&["5"]), set(&["6", "7", "10"]));
        let new = [
            Candidate { key: 0usize, level: 0, members: &grown },
            Candidate { key: 1, level: 0, members: &shrunk },
            Candidate { key: 2, level: 0, members: &fresh },
        ];
        let matched = carry_over(&old, &new);
        assert_eq!(matched.get(&0).map(String::as_str), Some("rust"));
        // 5 shares 1 of 3 with bread (0.33) and 6,7,10 shares 2 of 4 (0.5): the latter continues it.
        assert_eq!(matched.get(&2).map(String::as_str), Some("bread"));
        assert!(!matched.contains_key(&1));
    }

    #[test]
    fn levels_and_reuse() {
        let a = set(&["1", "2", "3"]);
        let old = [Candidate { key: "t".to_string(), level: 1, members: &a }];
        let same = set(&["1", "2", "3"]);
        // Same members but a different level: not the same topic.
        let new = [Candidate { key: 0usize, level: 0, members: &same }];
        assert!(carry_over(&old, &new).is_empty());
        // Two identical new groups: only one inherits.
        let new = [Candidate { key: 0usize, level: 1, members: &same }, Candidate { key: 1, level: 1, members: &same }];
        assert_eq!(carry_over(&old, &new).len(), 1);
    }
}
