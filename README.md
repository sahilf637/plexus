# Plexus

**Copy anything. Your second brain wires itself.**

Plexus is a lightweight desktop app that turns whatever you copy into a connected knowledge graph.
Paste text, a link, a screenshot or a document, and Plexus links it to what you already have,
groups it into topics, and shows everything as a map you can explore and ask questions of.

- **Instant:** a paste lands on the map right away and is linked within about a second.
- **Any format:** text, code, web pages, images, PDFs and more.
- **Works offline:** linking and grouping run on your machine, with no setup and no account.
- **Any AI provider (optional):** connect one for titles, topics, explained links and answers.
- **Ask anything:** an assistant that searches and follows links across everything you've saved,
  and cites what it used.

## Status

Early design. Nothing to run yet. See [docs/OVERVIEW.md](docs/OVERVIEW.md) for the plan.

## Stack

[Tauri v2](https://v2.tauri.app) (Rust) + Svelte 5 + TypeScript + Vite, SQLite with sqlite-vec,
Sigma.js for the graph.

## License

[MIT](LICENSE)
