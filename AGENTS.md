# Plexus

Plexus is a lightweight desktop app that turns whatever you copy into a connected knowledge graph.
Paste anything (text, a link, a screenshot, a PDF), and Plexus files it, links it to what you
already have, groups it into topics, and shows it all as a map you can explore and ask questions
of. There is no editor to manage and no filing to do.

See [docs/OVERVIEW.md](docs/OVERVIEW.md) for the product and technical design.

## Status

Pre-scaffold. The repo holds only project files and the design docs. Update this section and
**Commands** below as soon as the app is scaffolded.

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
7. **Raw input is the source of truth.** Original pastes are never modified. Everything derived from
   them (chunks, embeddings, concepts, edges, clusters, layout) is a rebuildable cache tagged with
   the pipeline version that produced it.

## Architecture (planned)

```
Clipboard / hotkey / drag-drop
        │
        ▼
T0  <50ms  detect type → store raw → node appears (placeholder)
T1  <1s    extract text (local) → chunk → embed (local) → similarity edges → place in cluster
T2  bg     LLM: title, summary, concepts, typed edges with reasons (bounded context)
        │
        ▼
SQLite (raw items, chunks, vectors, FTS, concepts, edges, clusters, layout)
        │
        ▼
Svelte UI: graph map (Sigma.js), item reader, Ask agent
```

- **Frontend:** Svelte 5 + TypeScript + Vite. Graph rendering with Sigma.js + Graphology. Heavy graph
  algorithms (community detection, layout) run in a Web Worker.
- **Backend:** Tauri v2 (Rust). Keep it **thin and modular**: storage, extraction, embeddings, the
  ingest queue, LLM providers, and the command surface. The Rust side should stay easy to read
  for someone new to Rust.
- **Storage:** SQLite (`rusqlite`, bundled) + sqlite-vec for vectors + FTS5 for keyword search.
  One database file per knowledge base.
- **Embeddings:** small local model (bge-small) via ONNX Runtime, shipped with the app.
- **LLM providers:** OpenAI-compatible (Groq, OpenRouter, Ollama, LM Studio, …), Anthropic, Gemini.
  Keys live in the system keychain, never in files or the database.

### Planned layout

```
frontend/   Svelte + Vite UI
backend/    Rust / Tauri core
docs/       product and design docs
scripts/    build helpers (e.g. fetching the embedding model)
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
  reads full text only when it chooses to, within a token/step budget.

## Conventions

- Match the surrounding code's style, naming and comment density.
- Rust: `thiserror` for error types, no `unwrap()` outside tests, one module per concern.
- TypeScript: strict mode. All calls into Rust go through one typed API module in the frontend.
- UI copy uses plain language. Keyboard shortcuts are always shown next to their command.
- Every user-visible AI change is explainable (has a reason) and undoable.
- Keep docs current: update this file and `docs/OVERVIEW.md` when architecture or commands change.

## Prior art and licensing

- **DocBook** (`~/ideas/DocBook`, same author): source of the Tauri shell, capture/hotkeys,
  local embeddings, FTS search and the provider layer. Reuse freely.
- **Atomic** (github.com/kenforthewin/atomic, **MIT**): code may be adapted with attribution
  (keep their copyright notice in a `THIRD_PARTY_NOTICES` file).
- **LLM Wiki** (github.com/nashsu/llm_wiki, **GPL-3.0**): read for ideas only. **Do not copy code.**

## Commands

_To be filled in once the app is scaffolded._
