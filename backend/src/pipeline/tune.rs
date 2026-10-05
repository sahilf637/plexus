//! Measuring link quality on a seeded library (`npm run seed -- <folder>`), whose notes carry
//! their topic as `seed_topic:`. Embeds every note with the real model, then reports how
//! linking would turn out at a range of thresholds:
//!
//!   PLEXUS_SEED=<folder> cargo test tune_links -- --ignored --nocapture
//!
//! A link between notes on different topics isn't always wrong (caffeine and memory, heat pumps
//! and climate), so precision here is a floor; the printed samples show what those links are.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::embed::{Embedder, MODEL_DIR};
use super::{chunk, embedding_text};
use crate::capture::detect::Kind;
use crate::db::{Db, chunks, items};
use crate::library::Library;
use crate::library::reconcile::reconcile;

/// Neighbouring chunks looked at per chunk, as in linking.
const NEIGHBOURS_PER_CHUNK: usize = 40;

#[test]
#[ignore = "needs the model files and a seeded library; prints a report"]
fn tune_links() {
    let Some(seed) = std::env::var_os("PLEXUS_SEED").map(PathBuf::from) else {
        panic!("set PLEXUS_SEED to a library made by `npm run seed -- <folder>`");
    };
    let scratch = tempfile::tempdir().unwrap();
    let library = Library::open(seed).unwrap();
    let db = Db::open(&scratch.path().join("index.db")).unwrap();
    reconcile(&library, &db).unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
    let embedder = Embedder::new(Embedder::find([model]));

    // Embed every note (links are left out: their pages aren't read here).
    let mut topics: HashMap<String, String> = HashMap::new();
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut vectors: HashMap<String, Vec<Vec<f32>>> = HashMap::new();
    let started = Instant::now();
    let mut chunk_count = 0;
    for item in db.with(|conn| items::all(conn)).unwrap() {
        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        let Some(topic) = note.meta.extra.get("seed_topic").and_then(|v| v.as_str()) else { continue };
        if item.kind == Kind::Url {
            continue;
        }
        let texts = chunk::chunk(&embedding_text(&item, &note.body));
        let v = embedder.embed_passages(&texts).unwrap();
        db.with(|conn| chunks::replace(conn, &item.id, &texts, &v)).unwrap();
        chunk_count += texts.len();
        topics.insert(item.id.clone(), topic.to_string());
        titles.insert(item.id.clone(), item.title.clone());
        vectors.insert(item.id, v);
    }
    let n = topics.len();
    println!("embedded {n} notes ({chunk_count} chunks) in {:?}", started.elapsed());

    // Every note's best similarity to every nearby note, closest first.
    let mut candidates: HashMap<String, Vec<(String, f32)>> = HashMap::new();
    db.with(|conn| {
        for (id, vs) in &vectors {
            let mut neighbours = Vec::new();
            for v in vs {
                neighbours.extend(chunks::nearest(conn, v, NEIGHBOURS_PER_CHUNK)?);
            }
            let mut best: Vec<_> = chunks::best_per_item(neighbours, id).into_iter().collect();
            best.sort_by(|a, b| b.1.total_cmp(&a.1));
            candidates.insert(id.clone(), best);
        }
        Ok(())
    })
    .unwrap();

    let same = |a: &str, b: &str| topics[a] == topics[b];
    let mut all: Vec<(f32, bool)> =
        candidates.iter().flat_map(|(id, c)| c.iter().map(move |(o, s)| (*s, same(id, o)))).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let quantiles = |same_topic: bool| {
        let v: Vec<f32> = all.iter().filter(|(_, s)| *s == same_topic).map(|(x, _)| *x).collect();
        let q = |p: f32| v[((v.len() - 1) as f32 * p) as usize];
        format!("p10 {:.3}  median {:.3}  p90 {:.3}  max {:.3}", q(0.1), q(0.5), q(0.9), q(1.0))
    };
    println!("nearest-neighbour similarity, same topic:      {}", quantiles(true));
    println!("nearest-neighbour similarity, different topic: {}", quantiles(false));

    let table = |name: &str, scored: &HashMap<String, Vec<(String, f32)>>, thresholds: &[f32]| {
        println!("\n{name}\n  threshold top-k | links  avg/note  linked notes  same-topic");
        for max_links in [5, 8] {
            for &threshold in thresholds {
                let edges = simulate(scored, threshold, max_links);
                let correct = edges.iter().filter(|(a, b)| same(a, b)).count();
                let linked: BTreeSet<&String> = edges.iter().flat_map(|(a, b)| [a, b]).collect();
                println!(
                    "  {threshold:>9.3}   {max_links}   | {:>5}  {:>8.1}  {:>5} ({:>3.0}%)  {:>8.1}%",
                    edges.len(),
                    2.0 * edges.len() as f32 / n as f32,
                    linked.len(),
                    100.0 * linked.len() as f32 / n as f32,
                    100.0 * correct as f32 / edges.len().max(1) as f32,
                );
            }
        }
    };
    let steps = |from: f32, by: f32, count: usize| (0..count).map(|i| from + by * i as f32).collect::<Vec<_>>();
    table("raw similarity", &candidates, &steps(0.60, 0.02, 7));

    // Raw threshold, plus each note's single best match when it clears a lower floor, so short
    // notes aren't left on their own.
    println!("\nraw similarity + best match above a floor\n  threshold floor | links  avg/note  linked notes  same-topic");
    for threshold in [0.66, 0.68, 0.70] {
        for floor in [0.60, 0.62, 0.64] {
            let mut edges = simulate(&candidates, threshold, 5);
            for (id, c) in &candidates {
                if let Some((other, s)) = c.first().filter(|(_, s)| *s >= floor) {
                    let _ = s;
                    edges.insert(if id < other { (id.clone(), other.clone()) } else { (other.clone(), id.clone()) });
                }
            }
            let correct = edges.iter().filter(|(a, b)| same(a, b)).count();
            let linked: BTreeSet<&String> = edges.iter().flat_map(|(a, b)| [a, b]).collect();
            println!(
                "  {threshold:>9.2} {floor:>5.2} | {:>5}  {:>8.1}  {:>5} ({:>3.0}%)  {:>8.1}%",
                edges.len(),
                2.0 * edges.len() as f32 / n as f32,
                linked.len(),
                100.0 * linked.len() as f32 / n as f32,
                100.0 * correct as f32 / edges.len().max(1) as f32,
            );
        }
    }

    // Hub-aware (CSLS): a note that is close to everything (long encyclopedic text, code
    // snippets that share "code-ness") counts its matches for less. Each note's neighbourhood
    // density is its mean similarity to its closest HUB_K notes.
    for hub_k in [5, 10] {
        let density: HashMap<&String, f32> = candidates
            .iter()
            .map(|(id, c)| (id, c.iter().take(hub_k).map(|(_, s)| s).sum::<f32>() / hub_k.min(c.len()).max(1) as f32))
            .collect();
        let adjusted: HashMap<String, Vec<(String, f32)>> = candidates
            .iter()
            .map(|(id, c)| {
                let mut c: Vec<_> = c
                    .iter()
                    .filter_map(|(o, s)| Some((o.clone(), s - (density[id] + density.get(o)?) / 2.0)))
                    .collect();
                c.sort_by(|a, b| b.1.total_cmp(&a.1));
                (id.clone(), c)
            })
            .collect();
        table(&format!("hub-aware, density over {hub_k} neighbours"), &adjusted, &steps(-0.02, 0.01, 8));
    }

    // Tiers: (strong, related, maybe) cutoffs, each tier capped per note like linking does.
    println!("\ntiers (strong / related / maybe cutoffs)\n  cutoffs            | tier     links  per note  same-topic | notes with strong/related, with any");
    use super::link::Tier;
    for (strong, related, maybe) in [(Tier::STRONG, Tier::RELATED, Tier::MAYBE), (0.72, 0.66, 0.60), (0.74, 0.68, 0.62)] {
        let mut tiers: [BTreeSet<(String, String)>; 3] = Default::default();
        for (id, c) in &candidates {
            for (t, (lo, hi, cap)) in [(strong, 2.0, 5), (related, strong, 3), (maybe, related, 3)].into_iter().enumerate() {
                for (other, _) in c.iter().filter(|(_, s)| *s >= lo && *s < hi).take(cap) {
                    tiers[t].insert(if id < other { (id.clone(), other.clone()) } else { (other.clone(), id.clone()) });
                }
            }
        }
        // A pair keeps its strongest tier.
        let strong_set = tiers[0].clone();
        tiers[1].retain(|p| !strong_set.contains(p));
        let upper: BTreeSet<_> = tiers[0].union(&tiers[1]).cloned().collect();
        tiers[2].retain(|p| !upper.contains(p));
        let linked = |sets: &[&BTreeSet<(String, String)>]| {
            sets.iter().flat_map(|s| s.iter().flat_map(|(a, b)| [a, b])).collect::<BTreeSet<_>>().len()
        };
        for (t, name) in ["strong", "related", "maybe"].iter().enumerate() {
            let correct = tiers[t].iter().filter(|(a, b)| same(a, b)).count();
            let coverage = if t == 2 {
                format!("{:>3.0}% / {:>3.0}%", 100.0 * linked(&[&tiers[0], &tiers[1]]) as f32 / n as f32,
                    100.0 * linked(&[&tiers[0], &tiers[1], &tiers[2]]) as f32 / n as f32)
            } else { String::new() };
            println!(
                "  {strong:.2}/{related:.2}/{maybe:.2}   | {name:<7} {:>5}  {:>8.1}  {:>8.1}% | {coverage}",
                tiers[t].len(),
                2.0 * tiers[t].len() as f32 / n as f32,
                100.0 * correct as f32 / tiers[t].len().max(1) as f32,
            );
        }
    }

    let threshold = std::env::var("TUNE_THRESHOLD").ok().and_then(|t| t.parse().ok()).unwrap_or(super::link::Tier::RELATED);
    let edges = simulate(&candidates, threshold, 5);
    println!("\nlinks across topics at {threshold:.2} (strongest first):");
    let mut cross: Vec<_> = edges
        .iter()
        .filter(|(a, b)| !same(a, b))
        .map(|(a, b)| (candidates[a].iter().find(|(o, _)| o == b).map_or(0.0, |c| c.1), a, b))
        .collect();
    cross.sort_by(|x, y| y.0.total_cmp(&x.0));
    for (s, a, b) in cross.iter().take(25) {
        println!("  {s:.3}  [{}] {}  ↔  [{}] {}", topics[*a], titles[*a], topics[*b], titles[*b]);
    }
    let unlinked: Vec<_> = topics.keys().filter(|id| !edges.iter().any(|(a, b)| a == *id || b == *id)).collect();
    println!("\nunlinked at {threshold:.2}: {}", unlinked.len());
    for id in unlinked.iter().take(15) {
        println!("  [{}] {}", topics[*id], titles[*id]);
    }
}

/// The links linking would make: each note's closest `max_links` above `threshold`, both ways.
fn simulate(candidates: &HashMap<String, Vec<(String, f32)>>, threshold: f32, max_links: usize) -> BTreeSet<(String, String)> {
    let mut edges = BTreeSet::new();
    for (id, c) in candidates {
        for (other, _) in c.iter().filter(|(_, s)| *s >= threshold).take(max_links) {
            let pair = if id < other { (id.clone(), other.clone()) } else { (other.clone(), id.clone()) };
            edges.insert(pair);
        }
    }
    edges
}
