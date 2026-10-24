// Writes a made-up map of N notes (clusters, links, positions, a topic tree) for the browser-only
// UI, to see how the map behaves at a size no real library here reaches:
//
//   npm run scale-map -- 50000          then open http://localhost:1430/?map=map.local.json
//
// Shaped like what the backend sends: about 5 links per note in three tiers, clusters of about 60
// notes, a topic tree three levels deep. Written to frontend/public/map.local.json (not tracked).

import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

const n = Number(process.argv[2] ?? 10000)
if (!Number.isFinite(n) || n < 100) {
  console.error('usage: npm run scale-map -- <notes, at least 100>')
  process.exit(1)
}

// A small deterministic generator, so every run makes the same map.
let seed = 0x9e3779b9
const random = () => {
  seed ^= seed << 13
  seed ^= seed >>> 17
  seed ^= seed << 5
  return (seed >>> 0) / 4294967296
}
const gauss = () => (random() + random() + random() + random() - 2) * 1.732

const clusters = Math.max(15, Math.floor(n / 60))
const spread = Math.sqrt(n) * 10
const centres = Array.from({ length: clusters }, () => ({ x: gauss() * spread, y: gauss() * spread }))
const kinds = ['text', 'text', 'text', 'url', 'url', 'code', 'image', 'file']
const members = Array.from({ length: clusters }, () => [])

const items = []
const positions = []
for (let i = 0; i < n; i++) {
  const cluster = (i * 7919) % clusters
  const id = `n${String(i).padStart(7, '0')}`
  const kind = kinds[Math.floor(random() * kinds.length)]
  const day = new Date(Date.UTC(2024, 0, 1) + i * 600_000)
  items.push({
    id,
    path: `items/${day.getUTCFullYear()}/${String(day.getUTCMonth() + 1).padStart(2, '0')}/note-${i}.md`,
    kind,
    title: `Note ${i} about topic ${cluster}`,
    sourceUrl: kind === 'url' ? `https://example.com/${i}` : null,
    language: null,
    capturedAt: day.toISOString(),
    bodyHash: `sha256:${i}`,
    assetPath: null,
    assetHash: null,
    mime: null,
    size: null,
    status: 'linked',
    extracted: null,
    summary: null,
    // Only the mock reads these; the real map has no bodies.
    body: 'x',
    assetFile: null,
    attachments: [],
  })
  positions.push({ id, x: centres[cluster].x + gauss() * 40, y: centres[cluster].y + gauss() * 40 })
  members[cluster].push(id)
}

// Each note links to the next few in its cluster: two strong, two related, one maybe.
const edges = []
for (const ids of members) {
  ids.forEach((a, p) => {
    for (const [step, tier, weight] of [[1, 'strong', 0.8], [2, 'strong', 0.76], [3, 'related', 0.7], [4, 'related', 0.68], [6, 'maybe', 0.64]]) {
      const b = ids[p + step]
      if (b) edges.push({ src: a, dst: b, kind: 'similar', weight, tier, reason: `both mention topic ${p}` })
    }
  })
}

// Topics: a lowest topic per cluster, grouped by about six into middle topics, those into about 15 top ones.
const topics = []
const itemTopic = {}
const perMid = 6
const mids = Math.ceil(clusters / perMid)
const tops = Math.min(15, mids)
const count = new Map()
const add = (t) => (topics.push(t), count.set(t.id, 0))
for (let t = 0; t < tops; t++) add({ id: `top${t}`, parentId: null, level: 0, label: `Area ${t}`, labelBy: 'auto', summary: null })
for (let m = 0; m < mids; m++) add({ id: `mid${m}`, parentId: `top${m % tops}`, level: 1, label: `Group ${m}`, labelBy: 'auto', summary: null })
for (let c = 0; c < clusters; c++) {
  const mid = Math.floor(c / perMid)
  add({ id: `leaf${c}`, parentId: `mid${mid}`, level: 2, label: `Topic ${c}`, labelBy: 'auto', summary: null })
  for (const id of members[c]) {
    itemTopic[id] = `leaf${c}`
    for (const up of [`leaf${c}`, `mid${mid}`, `top${mid % tops}`]) count.set(up, count.get(up) + 1)
  }
}
for (const t of topics) t.count = count.get(t.id)

const out = resolve(import.meta.dirname, '..', 'frontend', 'public', 'map.local.json')
mkdirSync(resolve(out, '..'), { recursive: true })
const json = JSON.stringify({ items, edges, positions, concepts: [], topics: { topics, itemTopic } })
writeFileSync(out, json)
console.log(`Wrote ${n} notes, ${edges.length} links and ${topics.length} topics (${(json.length / 1e6).toFixed(1)} MB) to ${out}`)
