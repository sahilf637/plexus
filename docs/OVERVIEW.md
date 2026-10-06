# Plexus — Product & Tech Overview

## Context

People collect a lot of scattered material: snippets, links, screenshots, docs. Notes apps only
stay useful if you file things by hand, and the graph views they offer turn into an unreadable
hairball. Plexus removes the filing entirely: **you copy, it connects.** The main view is a map of
everything you've collected, grouped into topics, with links that explain themselves, plus an
assistant that can answer questions by exploring the whole collection.

Decisions so far:
- **Platform:** desktop app (Tauri), local-first. Not a hosted web app.
- **Not a notes editor.** Capture and explore, not write. (DocBook already covers editing.)
- **AI providers:** any (local or cloud), and the app must work without one.
- **Stack preferences:** Svelte frontend; keep the Rust side thin and modular.

## Landscape

| Product | What it does | Gap Plexus targets |
|---|---|---|
| **Atomic** (MIT, Tauri + Rust + sqlite-vec + Sigma) | Markdown notes auto-tagged, embedded and linked by similarity; tag-based canvas; wiki synthesis; scheduled reports; agentic chat; MCP | No clipboard/hotkey capture; markdown + URLs only; embeddings need a remote provider or Ollama; edges have no type or reason; chat agent capped at 10 steps with no graph walking; heavy (server sidecar, cloud) |
| **LLM Wiki** (GPL-3, Tauri) | LLM builds an interlinked wiki from documents | Wiki-first, not capture-first |
| **Recall** (hosted) | Save and summarize content; knowledge graph view | Cloud only |
| **Reor** (archived Mar 2026) | Local AI notes with similarity links | Editor-first; discontinued |
| **Smart Connections / InfraNodus** | Obsidian plugins: semantic neighbors, text-network gaps | Live inside Obsidian |

**Positioning:** Atomic is a knowledge base you curate. Plexus is one you just feed.

## 1. Core loop

```
Global hotkey (any app) ─► capture window ─► paste / drop / type anything ─► Enter
   (or: tray, Ctrl+V in the main window, opt-in "save clipboard directly", later clipboard watch)
                    │
T0  <50ms           detect type → secret check → write note → node appears
T1  <1s, local      extract text → chunk → embed → similarity edges → join a cluster
T2  background, LLM title, summary, concepts, typed edges with reasons
                    │
                    ▼
       Map (topics → sub-topics → items)   ·   Ask (agent with citations)
```

Each stage is useful on its own. Without an AI provider, T2 is skipped and the map still works,
with topics named from their most distinctive words.

## 2. Capture: any format

The hotkey (default **Ctrl+Alt+V**) opens a small capture window that is already created and
hidden, so it appears instantly. It takes whatever is pasted, dropped or typed, shows a preview
with the detected kind, saves on **Enter** and closes on **Esc**. After saving it stays for a
moment showing what the item linked to. Capture goes through the window's own paste/drop events,
which deliver text, HTML, images and files the same way on every OS (and the focused window is
allowed to read the clipboard on Wayland). Settings can switch the hotkey to "save the clipboard
directly" with a notification instead of the window.

| Clipboard / input | Handling |
|---|---|
| Plain text, code | Item as-is; detect code and language |
| Rich HTML | Convert to markdown; keep source URL |
| URL | Fetch + readable-article extraction; YouTube → transcript; GitHub → README |
| Image / screenshot | Local OCR; optional vision LLM in T2 |
| Files | PDF, DOCX, EPUB, markdown, plain text |
| Audio (later) | Local transcription (whisper.cpp) |
| Anything else | Stored raw with its type; never rejected |

**Safety:** captures pass a secret detector (key/token patterns, high-entropy strings) before
storage. Clipboard watching is opt-in and skips password-manager sources. Exact duplicates (hash)
are merged; near-duplicates (embedding similarity > ~0.95) are flagged.

## 3. Storage: files are the source of truth

Every item is one markdown **note file** in a library folder the user can see. The note holds the
metadata plus either the content itself or, for images and documents, the text extracted from
them and a reference to the original, which sits beside the note with the same name. The SQLite
database is only an index built from the folder: deleting it rebuilds it.

```
~/Plexus/                                  (default; changeable in Settings)
  items/2026/10/rust-ownership-rules-k3f9.md
  items/2026/10/q3-board-report-m2a1.md
  items/2026/10/q3-board-report-m2a1.pdf   ← original, same stem as its note
  .plexus/index.db                         ← index (chunks, vectors, edges, layout); rebuildable
```

```markdown
---
plexus: 1                      # note format version
id: 01JA7Q…                    # ULID; the database keys on this
kind: pdf                      # text | code | url | image | pdf | docx | epub | audio | file
title: Q3 Board Report
captured: 2026-10-04T21:40:00Z
asset: q3-board-report-m2a1.pdf
asset_hash: sha256:9f2c…
original_name: Q3 Report FINAL v2.pdf
mime: application/pdf
pages: 14
extracted: pdf-text@1          # what produced the body; "edited" once the user changes it
links: []                      # links the user made by hand
title_by: ai                   # only when an AI provider renamed the note ("you" once reverted)
title_was: Pasted note         # the title before that, to put back
---

<!-- page 1 -->
Q3 revenue grew 18%…
```

| Kind | Extra frontmatter | Body |
|---|---|---|
| `text`, `code` | `language` for code | The content (code in a fenced block) |
| `url` | `source`, `title`, `site` | The extracted article as markdown |
| `image` | `asset`, `mime`, `width`, `height` | Embedded image, then OCR text / AI description |
| `pdf`, `docx`, `epub` | `asset`, `mime`, `pages` | Extracted text with page markers for citations |
| `audio`, `video` (later) | `asset`, `duration` | Transcript with timestamps |
| Anything else | `asset`, `mime`, `size` | Empty; still on the map |

Rules:
- **File names:** title slug (≤ 60 chars) + the last 4 characters of the id. Writes go to a temp
  file and are renamed into place, so a crash never leaves half a note.
- **Originals are copied** into the library by default so it is self-contained. Very large files
  can be linked instead (`asset_path` absolute + `asset_hash`), and are flagged if they go missing.
- **Several originals with one note** (screenshots plus a note about them) are one item: the
  first is `asset`, the rest are listed under `attachments:` (`file`, `hash`, `original_name`,
  `mime`, `size`) and saved as `<stem>_2.png`, `<stem>_3.pdf`, … The body embeds or links each
  one, then the note. Without a note, each original is its own item.
- **Extracted text** sits after a `<!-- plexus:extracted -->` line at the end of the body; what
  comes before it is the user's own note. It is regenerated by a newer extractor only while
  `extracted:` names a tool (`text@1`, `html@1`, `pdf-text@1`, `docx-text@1`, `epub-text@1`,
  `ocr@1`; `none@1` for types nothing reads). Once the user edits it (`extracted: edited`)
  Plexus never overwrites it.
- **Hand-made links** are the `links:` list of the note they were made from; the index mirrors
  them in `user_links` and draws them as strong `user` edges.
- **Deleted items** move to `.plexus/trash/<batch>/` (kept 30 days) and can be restored; the trash
  is outside `items/`, so it is never indexed.
- **What lives where:** everything the user owns (content, originals, hand-made links) is in
  the files. Everything computed (chunks, vectors, similarity and AI edges, topics, layout) is in
  the database, tagged with the pipeline version that produced it.
- **Reconcile:** on startup the folder is compared with the database by modification time, size and
  hash; new or changed notes are re-indexed, removed ones dropped. A file watcher comes later,
  with in-app editing.
- **Duplicates:** the same body hash or asset hash is reported as "already saved", not stored twice.

## 4. Graph model

Three layers: **Items ↔ Concepts ↔ Topics.**

- **Items:** each capture. Long items are split into chunks; links are found between chunks and
  rolled up to the item.
- **Concepts:** a small, deduplicated vocabulary extracted by the LLM ("LLM", "LLMs" and "large
  language models" are one concept, with aliases).
- **Topics:** a variable-depth tree of clusters (Louvain on a graph of links, concepts and
  nearest neighbours, at a low resolution for the top level and a higher one to split big groups),
  built locally and named locally from the notes' own words, with the top two levels renamed by the
  LLM when one is connected (it only names, never decides membership). A new item joins the topic
  most of its links are in; the tree is rebuilt only when the library has grown a good deal, and
  topic ids, names and places carry over by matching on member overlap. A name you set is kept.
  See [plans/M3.md](plans/M3.md).

**Tiers:** every link found by meaning has a tier saying how sure it is: `strong` (≥ 0.72
similarity, up to 5 per item), `related` (≥ 0.67, up to 3) and `maybe` (≥ 0.62, up to 3). An item
can have many links in each. The map draws strong and related links by default ("Links: Strong ·
Related · All" in the header); hovering an item shows all of its links. Strong links pull hardest
in the layout, maybes barely. Files known only by their name never get more than `maybe`.

**Edges** carry: type (`similar` by meaning, `user` by hand, or one the AI gave: `supports`,
`contradicts`, `extends`, `example_of`, `related`), weight, a one-line reason, confidence,
provenance (`embedding | llm | user`) and status (`proposed` for everything Plexus finds;
`rejected` only for an AI link the user removed, so it never comes back). Linking is fully
automatic: there is no approval step, and strength tells the user what to trust. For AI links
`src` is the item the statement is about ("src extends dst"); the panel words it from either end
("extends" / "extended by"). When a pair has several links, readers get one: by hand, else AI (its
type and reason, with the stronger tier and weight), else by meaning.

**Structure: two axes.** The *tree* says where an item lives (Area → Topic → Sub-topic → Item), one
variable-depth tree built from clusters in M3 (depth follows library size; "subject" is just its
top level). The *mesh* says how items connect: typed item ↔ item links, and item → concept tags,
which cut across topics. Concepts are chips and a map highlight, not map nodes. Cluster
membership is never decided by the AI, only named by it.

### Data model (sketch)

```
items(id, path, kind, title, source_url, captured_at, body_hash, asset_path, asset_hash, mime, status, pipeline_version)
chunks(id, item_id, ord, text)              vec_chunks (sqlite-vec)   fts (FTS5)
concepts(id, name, aliases, summary)        item_concepts(item_id, concept_id, weight)
edges(src, dst, kind, weight, reason, confidence, provenance, status, pipeline_version)
topics(id, parent_id, level, label, label_by, summary, named_members, centroid)
topic_members(item_id, topic_id)            -- the lowest topic only; ancestors follow parent_id
layout(node_id, level, x, y)
```

## 5. AI context: never the whole corpus

Per new item, one enrichment call of roughly 3k tokens, **regardless of corpus size**:

1. Candidates without an LLM: vector top-k + keyword (BM25) + concept-name matches.
2. The call receives the item (or chunk summaries), ~10–20 candidate cards (id, title, one-line
   summary) and the ~30 most relevant concepts.
3. It returns structured JSON: title, summary, concepts (existing id or new), edges to candidate
   ids with type, reason and confidence. Edges to ids it wasn't shown are dropped.

Supporting rules: cache by content hash + pipeline version; cheap model for enrichment, stronger
model for topic naming and answers; prompt caching for static prompts; batch requests for bulk
imports. Topic summaries let global questions ("what have I collected about X?") be answered from
summaries instead of raw items.

## 6. Ask: an agent that explores

A Claude-Code-style loop over the knowledge base rather than single-shot RAG.

| Tool | Purpose |
|---|---|
| `search(query, mode, scope)` | Hybrid / semantic / keyword / exact (literal or regex), in the whole library or one topic |
| `read(item, offset, limit)` | Paged reading of an item |
| `neighbors(item, depth)` | Walk the graph, with each link's reason |
| `topics(topic?)` | Browse the hierarchy: top level, or one topic's summary, sub-topics and members |
| `answer(text, sources, effort)` | Ends the run |

How a question runs (details in [plans/M4.md](plans/M4.md)):

- **Retrieve first.** Local hybrid search gives the model about ten candidate cards before its
  first call, so easy questions are answered in one or two calls; the loop only runs for harder
  ones. Each turn the model returns one JSON step (a tool, or the answer), which works the same on
  every provider.
- **Effort sets the budget.** The first step says `lookup`, `explain` or `survey`: about 3, 8 or 12
  tool steps, a token budget, and an answer length to match (a sentence or two for a lookup, never
  more than about 1,500 tokens). A question that runs out of steps ends as a partial answer.
- **Short names, capped results.** Notes are `n1`, `n2`…, topics `t1`…; a name not shown is refused.
  Results are capped at about 1,500 tokens and fenced as data. Notes that look like secrets are
  invisible. The agent is read-only.
- **Citations that are checked.** A source is `{n, item, quote}`; it stands only if the note was
  shown and the quote really occurs in it. Others are dropped (and the rest renumbered); an answer
  whose sources all fail is marked unsupported, while one that cites nothing because the notes
  don't cover the question is not.
- **Errors.** A bad step is retried once; a provider error keeps the run so it can be resumed;
  stopping keeps what was found. Asks have their own daily limit (Settings → AI).
- **On the map** the agent's path shows only while the Ask panel is showing and the window is in
  front, and it steps aside whenever you use the map: faint rings on the notes it was shown, a
  line through the ones it read, a pulse on the latest step, and a ring on each cited note. A
  "Follow" toggle eases the view to the latest note when it is out of sight (never while you are
  using the map).
- **Without an AI provider** Ask shows what search finds, with a prompt to connect one.

## 7. Big graphs

- **Rendering:** Sigma.js (WebGL). Positions are computed once (embedding projection +
  ForceAtlas2 refinement in a Web Worker) and stored, so the map stays stable between sessions;
  new items are placed near their neighbors without moving everything else.
- **Keyboard:** arrow keys glide the view while held (Shift: faster), `+` / `-` zoom, `0` shows the
  whole map, `F` centres the selected item, `/` searches, `Esc` clears, `Del` deletes. The map keys
  are inactive in text fields, the side panel and dialogs.
- **Zoom:** scrolling or pinching zooms around the cursor and eases smoothly; zooming out stops a
  little past the whole map. Two fingers sliding up zoom in; Settings → General → Map switches to
  the mouse-wheel direction.
- **Understanding:** semantic zoom. Zoomed out shows the top-level topics as soft shaded regions
  with names; zooming in, a topic gives way to its sub-topics, then to the items (their own labels
  are hidden while topic names are showing). Clicking a name zooms to it and opens its panel. Focus mode shows one item's 1–2 hop neighborhood. Each item
  keeps only its top 3–5 edges by default; weak similarity edges are hidden; filters by time,
  source, type and concept.
- **Data:** the UI requests slices by zoom level and viewport, so the full graph is never loaded at
  once.

## 8. Usability

- First run works immediately: no account, no API key. "Connect an AI" is an upgrade.
- One action: press the hotkey anywhere, paste, Enter. Also tray and drag-drop.
- The home screen is the map, with plain-language prompts ("3 new connections", "Your *Rust* topic
  grew this week").
- Simple by default; power features (edge types, filters, prompts) behind a toggle.
- Every AI change is explainable and undoable.

## 9. Milestones

| | Scope |
|---|---|
| **M0** | Scaffold; hotkey + capture window; note files + index; text/code/URL captured fully, images/files stored raw; secret detection; duplicates; local embeddings; similarity edges; Sigma map. Fully offline. See [plans/M0.md](plans/M0.md). |
| **M1** | Curate and extract. Delete items (trash with undo); plain-language link reasons and clearer strength; hand-made links. Extractors: plain-text files (`.md`, `.txt`, code files) read as text, rich HTML → markdown, images (local PaddleOCR PP-OCRv6 small, ~30 MB, swappable for a provider's vision later), PDF/DOCX/EPUB text; clipboard watching (opt-in). Until an item has text it links by name only, capped at `maybe`. No accept/reject flow. See [plans/M1.md](plans/M1.md). |
| **M2** | LLM enrichment of new items: titles (marked AI, revertible), summaries, concepts, typed edges with reasons, image descriptions. OpenAI-compatible, Anthropic and Gemini providers; keys in the system keychain. See [plans/M2.md](plans/M2.md). |
| **M3** | Topics: a variable-depth tree built locally by graph clustering, named locally (better with AI), shown as regions that appear with zoom, kept stable as the library grows; plus the slow, capped "improve older notes" pass. See [plans/M3.md](plans/M3.md). |
| **M4** | Ask: an agent with five small tools, checked citations (the quote must really be in the note), effort-sized answers and its path shown live on the map. See [plans/M4.md](plans/M4.md). |
| **M5** | Scale: viewport slicing, incremental layout; imports (Obsidian, Notion, bookmarks); MCP server. |

## Open decisions

- Trademark check on the name before any public launch.
