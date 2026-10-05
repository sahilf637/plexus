//! Search by words and by meaning, combined: keyword matches (FTS5) and items whose chunks are
//! close to the query's vector, merged by reciprocal rank fusion.

use std::collections::HashMap;

use crate::db::{chunks, graph};
use crate::error::Result;
use crate::state::AppState;

/// Below this, bge-small matches for a short query are mostly noise.
const MEANING_THRESHOLD: f32 = 0.5;
const NEIGHBOURS: usize = 60;
/// The usual reciprocal rank fusion constant.
const RRF_K: f64 = 60.0;

pub fn hybrid(state: &AppState, query: &str, limit: usize) -> Result<Vec<String>> {
    let by_words = state.db.with(|conn| graph::search(conn, query, limit))?;
    let by_meaning = match state.pipeline.embedder.available() && !query.trim().is_empty() {
        true => {
            let vector = state.pipeline.embedder.embed_query(query.trim())?;
            let neighbours = state.db.with(|conn| chunks::nearest(conn, &vector, NEIGHBOURS))?;
            let mut best: Vec<(String, f32)> = chunks::best_per_item(neighbours, "")
                .into_iter()
                .filter(|(_, similarity)| *similarity >= MEANING_THRESHOLD)
                .collect();
            best.sort_by(|a, b| b.1.total_cmp(&a.1));
            best.into_iter().map(|(id, _)| id).collect()
        }
        false => Vec::new(),
    };
    Ok(fuse(&[by_words, by_meaning], limit))
}

/// Items found by both lists rise to the top; each list contributes 1 / (k + rank).
fn fuse(lists: &[Vec<String>], limit: usize) -> Vec<String> {
    let mut scores: HashMap<&str, f64> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut ranked: Vec<(&str, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.into_iter().take(limit).map(|(id, _)| id.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::fuse;

    #[test]
    fn found_both_ways_ranks_first() {
        let words = vec!["a".to_string(), "b".to_string()];
        let meaning = vec!["c".to_string(), "b".to_string()];
        assert_eq!(fuse(&[words, meaning], 10)[0], "b");
        assert_eq!(fuse(&[vec![], vec!["x".to_string()]], 10), vec!["x"]);
    }
}
