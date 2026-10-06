//! Measuring the grouping on a seeded library (`npm run seed -- <folder>`), whose notes carry
//! their topic as `seed_topic:`. Embeds and links every note with the real model, builds the
//! tree for a range of settings, and reports how well it recovers the seeded topics:
//!
//!   PLEXUS_SEED=<folder> cargo test tune_topics -- --ignored --nocapture

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::cluster::Params;
use super::{Inputs, compute_with, read_inputs};
use crate::capture::detect::Kind;
use crate::db::items::{self, Status};
use crate::db::{Db, chunks};
use crate::library::Library;
use crate::library::reconcile::reconcile;
use crate::pipeline::embed::{Embedder, MODEL_DIR};
use crate::pipeline::{chunk, embedding_text, link};

/// The seed topic most of a group's notes are on, and how many.
fn majority(members: &[String], seed: &HashMap<String, String>) -> (String, usize) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for id in members {
        if let Some(topic) = seed.get(id) {
            *counts.entry(topic.as_str()).or_insert(0) += 1;
        }
    }
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0))).map(|(t, c)| (t.to_string(), c)).unwrap_or_default()
}

pub(crate) fn load(seed: PathBuf) -> (Db, tempfile::TempDir, HashMap<String, String>) {
    let scratch = tempfile::tempdir().unwrap();
    let library = Library::open(seed).unwrap();
    let db = Db::open(&scratch.path().join("index.db")).unwrap();
    reconcile(&library, &db).unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
    let embedder = Embedder::new(Embedder::find(vec![model]));
    let mut seed_topic = HashMap::new();
    for item in db.with(|conn| items::all(conn)).unwrap() {
        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        let Some(topic) = note.meta.extra.get("seed_topic").and_then(|v| v.as_str()) else { continue };
        if item.kind == Kind::Url {
            continue;
        }
        let texts = chunk::chunk(&embedding_text(&item, &note.body));
        let vectors = embedder.embed_passages(&texts).unwrap();
        db.with(|conn| {
            chunks::replace(conn, &item.id, &texts, &vectors)?;
            link::link(conn, &item.id, &vectors, false)?;
            items::set_status(conn, &item.id, Status::Linked, link::PIPELINE_VERSION)
        })
        .unwrap();
        seed_topic.insert(item.id, topic.to_string());
    }
    (db, scratch, seed_topic)
}

#[test]
#[ignore = "needs the model files and a seeded library; prints a report"]
fn tune_topics() {
    let Some(seed) = std::env::var_os("PLEXUS_SEED").map(PathBuf::from) else {
        panic!("set PLEXUS_SEED to a library made by `npm run seed -- <folder>`");
    };
    let (db, _scratch, seed_topic) = load(seed);
    let inputs: Inputs = db.with(|conn| read_inputs(conn)).unwrap();
    let seeded = seed_topic.values().collect::<std::collections::HashSet<_>>().len();
    println!("{} notes in {seeded} seeded topics; {} edges", inputs.graph.ids.len(), inputs.graph.adj.iter().map(Vec::len).sum::<usize>() / 2);

    println!("\n top  child | top-level  purity  split-seeds | leaves  avg   max  purity | loose  depth");
    for top in [0.2, 0.3, 0.5, 0.8, 1.0] {
        for child in [1.0, 1.5, 2.0] {
            let params = Params { top_resolution: top, child_resolution: child, ..Params::default() };
            let (topics, members) = compute_with(&inputs, &params);
            let membership: HashMap<String, String> = members.into_iter().collect();
            let by_topic = crate::db::topics::members_with_descendants(&topics, &membership);
            let tops: Vec<&crate::db::topics::Topic> = topics.iter().filter(|t| t.level == 0 && t.id != super::LOOSE_ID).collect();
            let leaves: Vec<&crate::db::topics::Topic> =
                topics.iter().filter(|t| !topics.iter().any(|o| o.parent_id.as_deref() == Some(t.id.as_str())) && t.id != super::LOOSE_ID).collect();
            let purity = |set: &[&crate::db::topics::Topic]| {
                let (mut right, mut all) = (0, 0);
                for t in set {
                    let members: Vec<String> = by_topic[&t.id].iter().cloned().collect();
                    right += majority(&members, &seed_topic).1;
                    all += members.len();
                }
                100.0 * right as f64 / all.max(1) as f64
            };
            // How many seeded topics have most of their notes in one top-level topic.
            let split = {
                let mut split = 0;
                for topic in seed_topic.values().collect::<std::collections::HashSet<_>>() {
                    let notes: Vec<&String> = seed_topic.iter().filter(|(_, t)| *t == topic).map(|(id, _)| id).collect();
                    let best = tops.iter().map(|t| notes.iter().filter(|id| by_topic[&t.id].contains(**id)).count()).max().unwrap_or(0);
                    if (best as f64) < 0.7 * notes.len() as f64 {
                        split += 1;
                    }
                }
                split
            };
            let sizes: Vec<usize> = leaves.iter().map(|t| by_topic[&t.id].len()).collect();
            let loose = by_topic.get(super::LOOSE_ID).map_or(0, |s| s.len());
            let depth = topics.iter().map(|t| t.level).max().unwrap_or(0) + 1;
            println!(
                " {top:>3.1}  {child:>4.1} | {:>9}  {:>5.1}%  {:>11} | {:>6}  {:>3.0}  {:>4}  {:>5.1}% | {loose:>4}   {depth}",
                tops.len(),
                purity(&tops),
                split,
                leaves.len(),
                sizes.iter().sum::<usize>() as f64 / sizes.len().max(1) as f64,
                sizes.iter().max().unwrap_or(&0),
                purity(&leaves),
            );
        }
    }

    // The tree at the default settings, with the names it gets.
    let (topics, members) = compute_with(&inputs, &Params::default());
    let membership: HashMap<String, String> = members.into_iter().collect();
    let by_topic = crate::db::topics::members_with_descendants(&topics, &membership);
    println!("\nthe default tree:");
    fn show(topics: &[crate::db::topics::Topic], parent: Option<&str>, by_topic: &HashMap<String, std::collections::HashSet<String>>, seed: &HashMap<String, String>, depth: usize) {
        for t in topics.iter().filter(|t| t.parent_id.as_deref() == parent) {
            let members: Vec<String> = by_topic[&t.id].iter().cloned().collect();
            let (topic, count) = majority(&members, seed);
            println!("{}{} ({} notes; mostly {topic}, {:.0}%)", "  ".repeat(depth), t.label, members.len(), 100.0 * count as f64 / members.len() as f64);
            show(topics, Some(&t.id), by_topic, seed, depth + 1);
        }
    }
    show(&topics, None, &by_topic, &seed_topic, 0);
}

/// Builds the index of a seeded library in place (`<folder>/.plexus/index.db`): embeds and links
/// every note and builds its topics, so `npm run export-map` has real data to export.
///
///   PLEXUS_SEED=<folder> cargo test index_seed -- --ignored --nocapture
#[test]
#[ignore = "needs the model files and a seeded library; writes <folder>/.plexus/index.db"]
fn index_seed() {
    let Some(seed) = std::env::var_os("PLEXUS_SEED").map(PathBuf::from) else {
        panic!("set PLEXUS_SEED to a library made by `npm run seed -- <folder>`");
    };
    let library = Library::open(seed).unwrap();
    let db = Db::open(&library.index_path()).unwrap();
    reconcile(&library, &db).unwrap();
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
    let embedder = Embedder::new(Embedder::find(vec![model]));
    for item in db.with(|conn| items::all(conn)).unwrap() {
        if item.kind == Kind::Url {
            continue;
        }
        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        let texts = chunk::chunk(&embedding_text(&item, &note.body));
        let vectors = embedder.embed_passages(&texts).unwrap();
        db.with(|conn| {
            chunks::replace(conn, &item.id, &texts, &vectors)?;
            link::link(conn, &item.id, &vectors, false)?;
            items::set_status(conn, &item.id, Status::Linked, link::PIPELINE_VERSION)
        })
        .unwrap();
    }
    let inputs: Inputs = db.with(|conn| read_inputs(conn)).unwrap();
    let (topics, members) = compute_with(&inputs, &Params::default());
    db.with(|conn| crate::db::topics::replace_all(conn, &topics, &members)).unwrap();
    println!("indexed {} notes into {} topics", inputs.graph.ids.len(), topics.len());
}
