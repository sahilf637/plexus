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
    body,
    assetFile: null,
  }))
}
