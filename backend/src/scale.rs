//! Measuring how the index behaves on a big synthetic library, so scale work can be judged by
//! numbers. No model is needed: notes get made-up vectors that fall into clusters, like real
//! topics do. Run in release mode (the vector search is C code, and slow unoptimised):
//!
//!   PLEXUS_SCALE_N=50000 cargo test --release scale_report -- --ignored --nocapture
//!   PLEXUS_SCALE_N=20000 cargo test --release scale_reconcile -- --ignored --nocapture
//!
//! `PLEXUS_SCALE_DIR` says where the scratch index goes (default `backend/target/scale-data`; not
//! /tmp, which may be memory). `PLEXUS_SCALE_OUT=<file>` also writes the timings (ms) as JSON, to
//! compare before and after a change.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::params;

use crate::capture::detect::Kind;
use crate::db::items::{self, Item};
use crate::db::{Db, chunks, concepts, graph, topics as topic_store};
use crate::error::Result;
use crate::pipeline::{link, reason};

const DIM: usize = 384;

/// A small deterministic random generator (xorshift), so every run builds the same library.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// 0..1
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Roughly normal, mean 0, spread 1.
    fn gauss(&mut self) -> f32 {
        (self.unit() + self.unit() + self.unit() + self.unit() - 2.0) * 1.732
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn random_unit(rng: &mut Rng) -> Vec<f32> {
    normalized((0..DIM).map(|_| rng.gauss()).collect())
}

/// `centre` plus noise; two such vectors from one centre are about `1 / (1 + sigma² · 384)` alike.
fn near(centre: &[f32], sigma: f32, rng: &mut Rng) -> Vec<f32> {
    normalized(centre.iter().map(|c| c + sigma * rng.gauss()).collect())
}

/// A pronounceable made-up word of six letters (so reasons and keyword search have text to chew).
fn word(mut n: u64) -> String {
    const C: &[u8] = b"bdfgklmnprstvz";
    const V: &[u8] = b"aeiou";
    let mut s = String::new();
    for _ in 0..3 {
        s.push(C[(n % 14) as usize] as char);
        n /= 14;
        s.push(V[(n % 5) as usize] as char);
        n /= 5;
    }
    s
}

fn scale_n(default: usize) -> usize {
    std::env::var("PLEXUS_SCALE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn scratch() -> tempfile::TempDir {
    let base = std::env::var_os("PLEXUS_SCALE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("scale-data"));
    std::fs::create_dir_all(&base).unwrap();
    tempfile::tempdir_in(base).unwrap()
}

/// Peak memory of this process so far (Linux), in MB.
fn peak_memory_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<f64>().ok()? / 1024.0)
}

/// Timings as they are taken: printed at once, and kept for the JSON file.
#[derive(Default)]
struct Report {
    rows: BTreeMap<String, f64>,
}

impl Report {
    fn add(&mut self, name: &str, ms: f64, note: &str) {
        println!("{name:<44} {ms:>12.1} ms   {note}");
        self.rows.insert(name.to_string(), ms);
    }

    fn size(&mut self, name: &str, value: f64, unit: &str) {
        println!("{name:<44} {value:>12.1} {unit}");
        self.rows.insert(name.to_string(), value);
    }

    /// Runs `f` once and records how long it took.
    fn once<T>(&mut self, name: &str, note: &str, f: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = f();
        self.add(name, started.elapsed().as_secs_f64() * 1000.0, note);
        out
    }

    /// Runs `f` for each of `runs` and records the average and the slowest.
    fn each<T>(&mut self, name: &str, runs: usize, mut f: impl FnMut(usize) -> T) {
        let mut times = Vec::new();
        for i in 0..runs {
            let started = Instant::now();
            f(i);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let average = times.iter().sum::<f64>() / times.len().max(1) as f64;
        let slowest = times.iter().cloned().fold(0.0, f64::max);
        self.add(name, average, &format!("average of {runs}, slowest {slowest:.1} ms"));
    }

    fn save(&self) {
        if let Some(path) = std::env::var_os("PLEXUS_SCALE_OUT") {
            std::fs::write(path, serde_json::to_string_pretty(&self.rows).unwrap()).unwrap();
        }
    }
}

fn percentile(sorted: &[Duration], share: f64) -> Duration {
    sorted.get(((sorted.len() as f64 * share) as usize).min(sorted.len().saturating_sub(1))).copied().unwrap_or_default()
}

/// A few notes to run operations on, with the vectors they were made with.
struct Sample {
    id: String,
    vectors: Vec<Vec<f32>>,
    text: String,
}

struct Corpus {
    samples: Vec<Sample>,
    chunk_count: usize,
    edge_count: usize,
    concept_count: usize,
}

/// Fills the index with `n` notes in clusters of about 60: bodies, chunks with vectors, links
/// of every tier (about 5 per note), concepts and positions. It is written in one transaction
/// (saving 100k notes one by one would take an hour); what a real save costs is measured on top
/// of the full library, see `scale_report`.
fn build(db: &Db, n: usize) -> Corpus {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let clusters = (n / 60).max(15);
    let centres: Vec<Vec<f32>> = (0..clusters).map(|_| random_unit(&mut rng)).collect();
    let mut members: Vec<Vec<String>> = vec![Vec::new(); clusters];
    let mut samples = Vec::new();
    let mut chunk_count = 0;
    let mut edge_count = 0;
    let mut concept_count = 0;
    db.with(|conn| -> Result<()> {
        let tx = conn.transaction()?;
        {
            let mut item = tx.prepare(
                "INSERT INTO items (id, path, kind, title, captured_at, body_hash, file_mtime, file_size, status, pipeline_version)
                 VALUES (?1, ?2, 'text', ?3, ?4, ?5, 1, 10, 'linked', ?6)",
            )?;
            let mut fts = tx.prepare("INSERT INTO items_fts (item_id, title, body) VALUES (?1, ?2, ?3)")?;
            let mut chunk = tx.prepare("INSERT INTO chunks (item_id, ord, text) VALUES (?1, ?2, ?3)")?;
            let mut vector = tx.prepare("INSERT INTO vec_chunks (rowid, embedding) VALUES (?1, ?2)")?;
            for i in 0..n {
                let cluster = (i * 7919) % clusters;
                let id = ulid::Ulid::new().to_string();
                let chunk_total = match rng.below(10) {
                    0..=4 => 1,
                    5..=7 => 3,
                    _ => 8,
                };
                let vocabulary = |rng: &mut Rng| {
                    if rng.unit() < 0.4 {
                        word(100_000 + (cluster * 50 + rng.below(50)) as u64)
                    } else {
                        word((rng.unit().powi(3) * 2000.0) as u64)
                    }
                };
                let texts: Vec<String> = (0..chunk_total).map(|_| (0..120).map(|_| vocabulary(&mut rng)).collect::<Vec<_>>().join(" ")).collect();
                let centre = near(&centres[cluster], 0.029, &mut rng);
                let vectors: Vec<Vec<f32>> = (0..chunk_total).map(|_| near(&centre, 0.012, &mut rng)).collect();
                let title = format!("{} {} notes {i}", word(100_000 + (cluster * 50) as u64), word(100_001 + (cluster * 50) as u64));
                let captured = format!("2025-{:02}-{:02}T{:02}:{:02}:00Z", i % 12 + 1, i % 27 + 1, i % 24, i % 60);
                item.execute(params![id, format!("items/2025/{:02}/note-{i}.md", i % 12 + 1), title, captured, format!("h{i}"), link::PIPELINE_VERSION])?;
                fts.execute(params![id, title, texts.join("\n\n")])?;
                for (ord, (text, v)) in texts.iter().zip(&vectors).enumerate() {
                    chunk.execute(params![id, ord as i64, text])?;
                    vector.execute(params![tx.last_insert_rowid(), chunks::to_blob(v)])?;
                }
                chunk_count += chunk_total;
                if i % (n / 20).max(1) == 0 {
                    samples.push(Sample { id: id.clone(), vectors, text: texts.join("\n") });
                }
                members[cluster].push(id);
            }

            let mut edge = tx.prepare(
                "INSERT OR IGNORE INTO edges (src, dst, kind, weight, reason, provenance, status, pipeline_version, created_at, tier)
                 VALUES (?1, ?2, 'similar', ?3, ?4, 'embedding', 'proposed', ?5, 'now', ?6)",
            )?;
            // Each note links to the next few in its cluster: two strong, two related, one maybe.
            for (c, ids) in members.iter().enumerate() {
                for (p, a) in ids.iter().enumerate() {
                    for (step, weight, tier) in [(1, 0.80, "strong"), (2, 0.76, "strong"), (3, 0.70, "related"), (4, 0.68, "related"), (6, 0.64, "maybe")] {
                        let Some(b) = ids.get(p + step) else { continue };
                        let (x, y) = if a < b { (a, b) } else { (b, a) };
                        let reason = format!("both mention {}, {}", word(100_000 + (c * 50) as u64), word(100_002 + (c * 50) as u64));
                        edge_count += edge.execute(params![x, y, weight, reason, link::PIPELINE_VERSION, tier])?;
                    }
                }
            }
            // About one concept for every eight notes, each tagging four notes of one cluster.
            let per_cluster = (n / 8 / clusters).max(2);
            let mut concept = tx.prepare("INSERT INTO concepts (id, name, embedding, created_at) VALUES (?1, ?2, ?3, 'now')")?;
            let mut tag = tx.prepare("INSERT OR IGNORE INTO item_concepts (item_id, concept_id, weight) VALUES (?1, ?2, 1)")?;
            let mut layout = tx.prepare("INSERT INTO layout (item_id, x, y) VALUES (?1, ?2, ?3)")?;
            let spread = (n as f32).sqrt() * 10.0;
            for (c, ids) in members.iter().enumerate() {
                let (cx, cy) = (rng.gauss() * spread, rng.gauss() * spread);
                for k in 0..per_cluster {
                    concept_count += 1;
                    concept.execute(params![concept_count as i64, format!("concept {c}-{k}"), chunks::to_blob(&near(&centres[c], 0.05, &mut rng))])?;
                }
                for (p, id) in ids.iter().enumerate() {
                    for t in 0..4 {
                        let concept_id = (concept_count - per_cluster + (p + t * 3) % per_cluster + 1) as i64;
                        tag.execute(params![id, concept_id])?;
                    }
                    layout.execute(params![id, cx + rng.gauss() * 40.0, cy + rng.gauss() * 40.0])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    Corpus { samples, chunk_count, edge_count, concept_count }
}

#[test]
#[ignore = "builds a big synthetic library; prints a report"]
fn scale_report() {
    let n = scale_n(20_000);
    let dir = scratch();
    let db = Arc::new(Db::open(&dir.path().join("index.db")).unwrap());
    let mut report = Report::default();
    println!("\n=== {n} notes ===");
    let built = Instant::now();
    let corpus = build(&db, n);
    report.add("build: whole library", built.elapsed().as_secs_f64() * 1000.0, "");
    let file_mb = std::fs::metadata(dir.path().join("index.db")).map(|m| m.len() as f64 / 1e6).unwrap_or(0.0);
    report.size("library: notes", n as f64, "");
    report.size("library: chunks (vectors)", corpus.chunk_count as f64, "");
    report.size("library: links", corpus.edge_count as f64, "");
    report.size("library: concepts", corpus.concept_count as f64, "");
    report.size("library: index file", file_mb, "MB");
    println!();

    let mut rng = Rng(42);

    // --- saving a note into the full library ------------------------------------------------
    // The first step is what capture waits for (under 50 ms is the promise); the second is the
    // background work after it.
    let fresh: Vec<(Item, String, Vec<String>, Vec<Vec<f32>>)> = (0..30)
        .map(|i| {
            let text = (0..120).map(|_| word((rng.unit().powi(3) * 2000.0) as u64)).collect::<Vec<_>>().join(" ");
            let mut item = items::sample(&format!("fresh{i:03}"), &format!("items/2026/01/fresh-{i}.md"), &format!("fresh-hash-{i}"));
            item.title = format!("Fresh note {i}");
            (item, text.clone(), vec![text], vec![random_unit(&mut rng)])
        })
        .collect();
    report.each("save: write the note into the index (capture)", fresh.len(), |i| {
        db.with(|conn| items::upsert(conn, &fresh[i].0, &fresh[i].1)).unwrap();
    });
    report.each("save: store its chunks and vectors", fresh.len(), |i| {
        db.with(|conn| chunks::replace(conn, &fresh[i].0.id, &fresh[i].2, &fresh[i].3)).unwrap();
    });

    // --- search --------------------------------------------------------------------------
    report.each("search: nearest 40 chunks (one scan)", 20, |_| {
        let q = random_unit(&mut rng);
        db.with(|conn| chunks::nearest(conn, &q, 40)).unwrap();
    });
    report.each("search: keyword, common word", 20, |_| {
        db.with(|conn| graph::search(conn, &word(3), 50)).unwrap();
    });
    report.each("search: keyword, rare word", 20, |_| {
        db.with(|conn| graph::search(conn, &word(100_000 + 7), 50)).unwrap();
    });
    report.each("search: how many notes hold a common word", 5, |_| {
        db.with(|conn| chunks::items_with_word(conn, &word(3))).unwrap();
    });
    // Ask's exact search reads every body until its time box (1.5 s) runs out.
    report.once("ask: exact search, one pass over every body", "", || {
        let regex = regex::Regex::new("zzzzzzzz").unwrap();
        db.with(|conn| {
            let mut stmt = conn.prepare("SELECT item_id, body FROM items_fts")?;
            let mut rows = stmt.query([])?;
            let mut seen = 0;
            while let Some(row) = rows.next()? {
                let body: String = row.get(1)?;
                regex.find(&body);
                seen += 1;
            }
            Ok(seen)
        })
        .unwrap()
    });

    // --- linking a new note --------------------------------------------------------------
    // While it runs, another thread asks for one note, as the UI does: how long does it wait?
    let stop = Arc::new(AtomicBool::new(false));
    let probe = {
        let (db, stop, id) = (db.clone(), stop.clone(), corpus.samples[0].id.clone());
        std::thread::spawn(move || {
            let mut waits = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let started = Instant::now();
                db.with(|conn| items::get(conn, &id)).unwrap();
                waits.push(started.elapsed());
                std::thread::sleep(Duration::from_millis(2));
            }
            waits
        })
    };
    let mut linked = Vec::new();
    report.each("link: find and store a note's links", corpus.samples.len().min(10), |i| {
        let s = &corpus.samples[i];
        linked.push(db.with(|conn| link::link(conn, &s.id, &s.vectors, false)).unwrap());
        // The real pipeline embeds the next note in between; the lock is free then.
        std::thread::sleep(Duration::from_millis(20));
    });
    report.each("link: write the links' reasons", corpus.samples.len().min(10), |i| {
        let s = &corpus.samples[i];
        db.with(|conn| reason::explain(conn, &s.id, &s.text, &linked[i], false)).unwrap();
    });
    stop.store(true, Ordering::Relaxed);
    let mut waits = probe.join().unwrap();
    waits.sort();
    report.add("while linking: UI read waits, median", percentile(&waits, 0.5).as_secs_f64() * 1000.0, &format!("{} reads", waits.len()));
    report.add("while linking: UI read waits, slowest", percentile(&waits, 1.0).as_secs_f64() * 1000.0, "");
    println!();

    // --- topics --------------------------------------------------------------------------
    report.once("topics: rebuild the whole tree", "", || crate::topics::rebuild_in(&db).unwrap());
    report.each("topics: place one new note (assign)", 20, |i| {
        let s = &corpus.samples[i % corpus.samples.len()];
        db.with(|conn| crate::topics::assign(conn, &s.id)).unwrap();
    });
    report.each("topics: what assign loads (all + memberships)", 5, |_| {
        db.with(|conn| {
            let all = topic_store::all(conn)?;
            let membership = topic_store::memberships(conn)?;
            Ok(topic_store::members_with_descendants(&all, &membership).len())
        })
        .unwrap();
    });
    report.once("topics: count linked notes (per linked note)", "", || db.with(|conn| conn.query_row("SELECT COUNT(*) FROM items WHERE status = 'linked'", [], |r| r.get::<_, i64>(0)).map_err(Into::into)).unwrap());
    println!();

    // --- what the map asks for at start ---------------------------------------------------
    let all_items = report.once("get_map: items", "", || db.with(|conn| items::all(conn)).unwrap());
    let all_edges = report.once("get_map: links", "", || db.with(|conn| graph::edges(conn)).unwrap());
    let positions = report.once("get_map: positions", "", || db.with(|conn| graph::positions(conn)).unwrap());
    let concept_items = report.once("get_map: concepts with their notes", "", || db.with(|conn| concepts::with_items(conn)).unwrap());
    let topics = report.once("get_map: topics", "", || db.with(|conn| crate::topics::data(conn)).unwrap());
    let mb = |bytes: usize| bytes as f64 / 1e6;
    let sizes = [
        ("items", mb(serde_json::to_vec(&all_items).unwrap().len())),
        ("links", mb(serde_json::to_vec(&all_edges).unwrap().len())),
        ("positions", mb(serde_json::to_vec(&positions).unwrap().len())),
        ("concepts", mb(serde_json::to_vec(&concept_items).unwrap().len())),
        ("topics", mb(serde_json::to_vec(&topics).unwrap().len())),
    ];
    for (name, size) in sizes {
        report.size(&format!("get_map: JSON size, {name}"), size, "MB");
    }
    report.size("get_map: JSON size, total", sizes.iter().map(|s| s.1).sum(), "MB");
    report.once("save_layout: save every position", "", || {
        db.with(|conn| graph::save_positions(conn, &positions)).unwrap();
    });
    println!();

    // --- background jobs -------------------------------------------------------------------
    report.once("start: load every note (pipeline prepare)", "", || {
        db.with(|conn| Ok(items::all(conn)?.iter().filter(|i| crate::extract::worker::needs_extraction(i)).count())).unwrap()
    });
    report.each("enrich: all concepts, compared with one vector", 5, |_| {
        let q = random_unit(&mut rng);
        db.with(|conn| Ok(concepts::all_embedded(conn)?.iter().map(|(_, e)| concepts::similarity(&q, e)).fold(0.0, f32::max))).unwrap();
    });
    report.each("older notes: pick the next one", 3, |_| {
        let ai = crate::settings::AiSettings {
            enabled: true,
            improve_old: true,
            improve_per_day: 0,
            connections: vec![crate::settings::AiConnection { id: "c".into(), ..Default::default() }],
            active: "c".into(),
            ..Default::default()
        };
        db.with(|conn| crate::enrich::older::pick(conn, &ai)).unwrap();
    });
    report.once("topics: read all vectors (for a rebuild)", "", || db.with(|conn| chunks::item_vectors(conn)).unwrap().len());
    if let Some(peak) = peak_memory_mb() {
        report.size("process: peak memory", peak, "MB");
    }
    report.save();
}

/// Reconcile on a folder of `n` real note files: first import, nothing changed, and every file
/// touched (as a sync tool or a copy to another computer would).
#[test]
#[ignore = "writes a big folder of notes; prints a report"]
fn scale_reconcile() {
    use crate::library::Library;
    use crate::library::note::{FORMAT_VERSION, Note, NoteMeta};
    use crate::library::reconcile::reconcile;

    let n = scale_n(10_000);
    let dir = scratch();
    let library = Library::open(dir.path().join("lib")).unwrap();
    let db = Db::open(&library.index_path()).unwrap();
    let mut report = Report::default();
    println!("\n=== reconcile, {n} note files ===");
    let started = Instant::now();
    let mut rng = Rng(7);
    for i in 0..n {
        let body = (0..80).map(|_| word((rng.unit().powi(3) * 5000.0) as u64)).collect::<Vec<_>>().join(" ");
        let meta = NoteMeta {
            plexus: FORMAT_VERSION,
            id: ulid::Ulid::new().to_string(),
            kind: Kind::Text,
            title: format!("Note {i}"),
            captured: "2025-05-05T10:00:00Z".into(),
            source: None,
            language: None,
            hash: crate::library::text_hash(&body),
            asset: None,
            asset_hash: None,
            original_name: None,
            mime: None,
            size: None,
            extracted: None,
            title_by: None,
            title_was: None,
            attachments: Vec::new(),
            links: Vec::new(),
            extra: BTreeMap::new(),
        };
        let path = library.items_dir().join(format!("2025/{:02}", i % 12 + 1)).join(format!("note-{i}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, Note { meta, body }.to_file().unwrap()).unwrap();
    }
    println!("(wrote the folder in {:?})", started.elapsed());

    report.once("reconcile: first import", "every note is new", || reconcile(&library, &db).unwrap());
    report.once("reconcile: nothing changed", "start of the app", || reconcile(&library, &db).unwrap());
    let later = std::time::SystemTime::now() + Duration::from_secs(60);
    for path in crate::library::reconcile::note_files(&library.items_dir()) {
        std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
    }
    let touched = report.once("reconcile: every file touched", "known notes whose files changed", || reconcile(&library, &db).unwrap());
    println!("({touched:?})");
    if let Some(peak) = peak_memory_mb() {
        report.size("process: peak memory", peak, "MB");
    }
    report.save();
}
