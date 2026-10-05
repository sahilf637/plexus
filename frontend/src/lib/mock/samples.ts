// A handful of items so `npm run dev:ui` shows a populated map.

import type { ItemDetail, Kind } from '../types'

const samples: [Kind, string, string][] = [
  ['text', 'Ownership rules in Rust', 'Each value has a single owner; when the owner goes out of scope the value is dropped.'],
  ['code', 'fn longest<\'a>(x: &\'a str, y: &\'a str) -> &\'a str', '```rust\nfn longest<\'a>(x: &\'a str, y: &\'a str) -> &\'a str {\n    if x.len() > y.len() { x } else { y }\n}\n```'],
  ['url', 'doc.rust-lang.org/book/ch04-01-what-is-ownership', 'https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html'],
  ['text', 'Sourdough starter feeding schedule', 'Feed 1:1:1 flour, water and starter every 12 hours at room temperature.'],
  ['text', 'Why bread needs steam', 'Steam keeps the crust soft early in the bake so the loaf can expand fully.'],
  ['text', 'Meeting notes: Q4 planning', 'Ship the capture window first; graph view next; hire one designer.'],
  ['image', 'Whiteboard sketch', ''],
  ['file', 'Q3 board report', ''],
  ['text', 'Spaced repetition', 'Reviewing at growing intervals beats cramming for long-term memory.'],
  ['url', 'en.wikipedia.org/wiki/Forgetting_curve', 'https://en.wikipedia.org/wiki/Forgetting_curve'],
]

/** What the AI would have said about the samples, so the map shows concepts and summaries. */
const summaries = [
  'Rust gives each value one owner and frees it when the owner goes out of scope.',
  'A function that returns the longer of two string slices, with explicit lifetimes.',
  'The Rust book chapter on how ownership works.',
  'How often to feed a sourdough starter, and in what proportions.',
  'Steam keeps the crust soft early on so the loaf can rise fully.',
  null,
  null,
  null,
  'Reviewing at growing intervals beats cramming for lasting memory.',
  'How memory fades over time without review.',
]

/** Concepts by sample index. */
export const sampleConcepts: Record<string, number[]> = {
  ownership: [0, 1, 2],
  'borrow checker': [0, 1],
  'sourdough starter': [3, 4],
  'bread baking': [3, 4],
  'spaced repetition': [8, 9],
  memory: [8, 9],
}

export function sampleItems(): ItemDetail[] {
  const start = Date.parse('2026-10-01T09:00:00Z')
  return samples.map(([kind, title, body], i) => ({
    id: `sample-${i}`,
    path: `items/2026/10/sample-${i}.md`,
    kind,
    title,
    sourceUrl: kind === 'url' ? body : null,
    language: kind === 'code' ? 'rust' : null,
    capturedAt: new Date(start + i * 3_600_000).toISOString(),
    bodyHash: `sample-${i}`,
    assetPath: null,
    assetHash: null,
    mime: kind === 'image' ? 'image/png' : kind === 'file' ? 'application/pdf' : null,
    size: null,
    status: 'linked',
    extracted: kind === 'url' ? 'web@1' : null,
    summary: summaries[i] ?? null,
    titleByAi: false,
    titleWas: null,
    body,
    assetFile: null,
    attachments: [],
  }))
}
