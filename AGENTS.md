# Plexus

Plexus is a lightweight desktop app that turns whatever you copy into a connected knowledge graph.
Paste anything (text, a link, a screenshot, a PDF), and Plexus files it, links it to what you
already have, groups it into topics, and shows it all as a map you can explore and ask questions
of. There is no editor to manage and no filing to do.

See [docs/OVERVIEW.md](docs/OVERVIEW.md) for the product and technical design.

## Status

M0 to M4 are done ([M0](docs/plans/M0.md), [M1](docs/plans/M1.md), [M2](docs/plans/M2.md), [M3](docs/plans/M3.md), [M4](docs/plans/M4.md)):
capture, local linking with plain-language reasons, deleting with undo, hand-made links, text
extraction (files, HTML, PDF/DOCX/EPUB, OCR), opt-in clipboard watching, and optional AI
enrichment of new items (titles, summaries, concepts, typed links, image descriptions). Linking
is fully automatic; the only thing a user removes is an AI link or an AI title (undo of an AI
change). M3 adds topics: a tree of groups built locally from the links, named locally (better with
AI), shown on the map as regions that change with the zoom, kept stable as the library grows, plus
an optional slow pass that improves older notes. M4 adds Ask: a read-only agent that answers a
question from the notes with five small tools, checked citations and its path shown on the map.
Next: M5 (scale, imports, MCP).

## Principles

These decide trade-offs. When two options conflict, the one that better serves these wins.

1. **Instant.** A paste shows up on the graph in under 50 ms, and is linked by meaning within about
   a second, using local models only. LLM work happens later in the background and never blocks
   the UI.
2. **Any format.** Whatever is on the clipboard is handled: text, code, rich HTML, URLs, images,
   files. Unsupported input is stored as-is and never rejected.
3. **Works with no AI provider.** Local embeddings, similarity links and clusters work offline with
   zero setup. Connecting an LLM adds titles, concepts, and typed, explained links on top.
4. **Bounded AI context.** No AI call ever receives the whole corpus. Every call has a fixed token
   budget built from retrieved candidates and summaries (see "AI context rules").
5. **Easy for everyone.** One action (paste), plain words in the UI (no "atoms", "embeddings" or
   "chunks"), power features behind a toggle.
6. **Lightweight.** One Tauri process, no server sidecar, small install, near-zero idle CPU.
7. **Files are the source of truth.** Every item is one markdown note in the user's library folder
   (plus its original file, for images and documents). The SQLite database is an index built from
   the folder and must be fully rebuildable from it. Everything derived (chunks, embeddings,
   concepts, edges, topics, layout) lives in the database, tagged with the pipeline version that
   produced it. See "Storage" in [docs/OVERVIEW.md](docs/OVERVIEW.md).

## Architecture (planned)

```
Global hotkey → capture window (paste / drop / type) → Enter
        │
        ▼
T0  <50ms  detect type → secret check → write note (+ original) → node appears
T1  <1s    extract text (local) → chunk → embed (local) → similarity edges → place in cluster
T2  bg     LLM: title, summary, concepts, typed edges with reasons (bounded context)
        │
        ▼
Library folder (note files + originals)  ──reconcile──►  SQLite index
                                                         (chunks, vectors, FTS, edges, layout)
        │
        ▼
Svelte UI: capture window, graph map (Sigma.js), item reader, Ask agent
```

- **Frontend:** Svelte 5 + TypeScript + Vite. Graph rendering with Sigma.js + Graphology. Heavy graph
  algorithms (community detection, layout) run in a Web Worker.
- **Backend:** Tauri v2 (Rust). Keep it **thin and modular**: storage, extraction, embeddings, the
  ingest queue, LLM providers, and the command surface. The Rust side should stay easy to read
  for someone new to Rust.
- **Storage:** note files in the library folder (default `~/Plexus/items/YYYY/MM/`), indexed by
  SQLite (`rusqlite`, bundled) + sqlite-vec for vectors + FTS5 for keyword search, in
  `<library>/.plexus/index.db`.
- **Embeddings:** bge-small-en-v1.5 (int8 ONNX) via `fastembed`, shipped with the app.
- **LLM providers:** OpenAI-compatible (Groq, OpenRouter, Ollama, LM Studio, …), Anthropic, Gemini.
  Several named **connections** can be saved (provider, address, model, key); one is in use. Each
  connection's key lives in the system keychain (`keyring`) under the connection's id, never in
  files or the database, and is removed with the connection. A key is never sent over plain
  `http://` except to this computer or a home network. Only new items
  are enriched (items saved after AI was switched on); `enrich::Lane::Backfill` is the low-priority
  lane a future "deeper linking" pass will use. A bad key pauses the queue; a daily call limit
  applies.

### Planned layout

```
frontend/              Svelte 5 + Vite; two entries: index.html (map), capture.html
  src/lib/api.ts       typed wrappers for Rust commands and events (the only place that calls Rust)
  src/lib/mock/        in-memory backend for `npm run dev:ui`
  src/lib/graph/       Sigma setup, reducers, layout worker, topic regions, the Ask agent's path (`path.ts`)
  src/lib/ask.svelte.ts  Ask state shared by the panel and the map
  src/capture/         capture window
  src/components/      map UI components
backend/               Tauri v2 crate `plexus`
  src/library/         note files: paths, frontmatter, originals, reconcile, trash, hand-made links
  src/db/              SQLite index: migrations, items, edges, search
  src/capture/         kind detection, secrets, dedupe, hotkeys, clipboard
  src/pipeline/        background worker: chunk, embed, link, plain-language link reasons
  src/llm/             AI providers (OpenAI-compatible, Anthropic, Gemini), one blocking JSON call; keychain keys
  src/enrich/          T2 queue and worker: bounded context, applying the answer, image descriptions,
                       the slow pass over older notes (`older.rs`)
  src/topics/          topic tree: graph, clustering (Louvain), matching, names (local and AI)
  src/ask/             the Ask agent: loop and budgets (`mod.rs`), prompt and step schema, the five tools
                       (`tools.rs`), citation checking (`cite.rs`), running and resuming questions (`session.rs`)
  src/fetch/           URL fetching + article extraction
  src/extract/         text out of files and images (text, HTML, PDF, DOCX, EPUB, OCR) in the background
docs/                  product and design docs; plans/ per milestone
scripts/               fetch-model, seed
```

## AI context rules

These are hard rules, not guidelines.

- An enrichment call for a new item gets: the item (or its chunk summaries if long), a fixed number
  of **candidate cards** (id, title, one-line summary) chosen by hybrid retrieval, and the most
  relevant existing concepts chosen by similarity. Never the full concept list or the full corpus.
- Edges the LLM proposes may only point at candidate ids it was shown; validate and drop the rest.
- Never send the same content version twice: cache by content hash + pipeline version.
- Run secret detection (API keys, tokens, high-entropy strings) on every capture **before** it is
  stored or sent anywhere. Flag secrets and keep them out of AI calls.
- The Ask agent works through tools that return compact results (ids, summaries, snippets) and
  reads full text only when it chooses to, within a token/step budget. It sees notes as `n1`, `n2`…
  and topics as `t1`…, never real ids; a name it wasn't shown is refused. Tool results are capped
  (about 1,500 tokens) and fenced as data, so text inside a note is never an instruction. It is
  read-only and has no network. Notes that look like secrets are invisible to every tool.
- An answer's sources stand only if the note was shown to the agent and the quote really occurs in
  it; the rest are dropped with their `[n]` marks. Asks have their own daily limit (`ai.askPerDay`).

## Conventions

- Match the surrounding code's style, naming and comment density.
- Rust: `thiserror` for error types, no `unwrap()` outside tests, one module per concern.
- TypeScript: strict mode. All calls into Rust go through one typed API module in the frontend.
- UI copy uses plain language. Keyboard shortcuts are always shown next to their command.
- Every user-visible AI change is explainable (has a reason) and undoable.
- Keep docs current: update this file and `docs/OVERVIEW.md` when architecture or commands change.
- **Commit messages are a single line** (no body, no trailers).

## Prior art and licensing

Plexus is its own codebase, written fresh.

- **Atomic** (github.com/kenforthewin/atomic, **MIT**): the main reference. Code may be adapted
  with attribution: keep their copyright notice in `THIRD_PARTY_NOTICES.md` and note the source
  file in a comment. Useful parts: markdown chunker, semantic-edge computation, provider traits,
  deferred graph maintenance, structured-output handling.
- **LLM Wiki** (github.com/nashsu/llm_wiki, **GPL-3.0**): read for ideas only. **Do not copy code.**

## Commands

Run from the repo root (npm workspace). Needs Node 20+, Rust 1.90+, and
[Tauri's system libraries](https://v2.tauri.app/start/prerequisites/).

```bash
npm install          # install all dependencies
npm run fetch-model  # download the bundled models: embeddings + OCR (also runs before dev/build)
npm run dev          # desktop app with hot reload (Vite on port 1430)
npm run dev:ui       # UI only, in a browser, against the in-memory mock backend
npm run check        # typecheck the frontend
npm run build        # production bundle
cd backend && cargo test && cargo clippy --all-targets
cd backend && cargo test -- --ignored --nocapture   # tests that run the real model (and the network)
```

Testing on a realistic library, without touching your own:

```bash
npm run seed -- /tmp/plexus-seed                    # ~230 notes across 15 topics, plus links
PLEXUS_LIBRARY=/tmp/plexus-seed npm run dev         # run the app on it (env var overrides Settings)
npm run export-map -- /tmp/plexus-seed              # its map, for `npm run dev:ui` at /?map=map.local.json
cd backend && PLEXUS_SEED=/tmp/plexus-seed cargo test tune_links -- --ignored --nocapture   # link quality report
cd backend && PLEXUS_SEED=/tmp/plexus-seed cargo test tune_topics -- --ignored --nocapture  # topic quality report
# Ask quality, with a real provider (also PLEXUS_ASK_KEY, PLEXUS_ASK_KIND=anthropic|gemini):
cd backend && PLEXUS_SEED=/tmp/plexus-seed PLEXUS_ASK_URL=https://api.openai.com/v1 PLEXUS_ASK_MODEL=gpt-4o-mini cargo test tune_ask -- --ignored --nocapture
cd backend && PLEXUS_SEED=/tmp/plexus-seed cargo test index_seed -- --ignored --nocapture   # build its index, topics included
```

`plexus --capture` opens the capture window (forwarded to the running instance);
`plexus --hidden` starts in the tray (used for start at login, which is skipped in debug builds).
