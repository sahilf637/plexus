// Exports a library's map (items with their text, links, layout) as JSON for the browser-only
// UI, so the map can be worked on with real data in `npm run dev:ui`:
//
//   npm run export-map -- <library folder>      then open http://localhost:1430/?map=map.local.json
//
// Reads the index Plexus built in <library>/.plexus/index.db (run the app on the library first).

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { DatabaseSync } from 'node:sqlite'

const library = process.argv[2]
if (!library) {
  console.error('usage: npm run export-map -- <library folder>')
  process.exit(1)
}
const root = resolve(library)
const db = new DatabaseSync(join(root, '.plexus', 'index.db'), { readOnly: true })

const body = (path) => {
  const text = readFileSync(join(root, path), 'utf8')
  const end = text.indexOf('\n---', 4)
  return end < 0 ? text : text.slice(end + 4).replace(/^\r?\n\r?\n?/, '')
}

const items = db
  .prepare(
    `SELECT id, path, kind, title, source_url AS sourceUrl, language, captured_at AS capturedAt,
            body_hash AS bodyHash, asset_path AS assetPath, asset_hash AS assetHash, mime, size,
            status, extracted
     FROM items ORDER BY captured_at`,
  )
  .all()
  .map((item) => ({ ...item, body: body(item.path), assetFile: null, attachments: [] }))
const edges = db.prepare("SELECT src, dst, kind, weight, tier, reason FROM edges WHERE status != 'rejected'").all()
const positions = db.prepare('SELECT item_id AS id, x, y FROM layout').all()

// Topics, if the index has them (built by the app, or `index_seed` in the backend tests).
let topics = { topics: [], itemTopic: {} }
try {
  const rows = db.prepare('SELECT id, parent_id AS parentId, level, label, label_by AS labelBy, summary FROM topics ORDER BY level, id').all()
  const itemTopic = Object.fromEntries(db.prepare('SELECT item_id, topic_id FROM topic_members').all().map((r) => [r.item_id, r.topic_id]))
  const parent = new Map(rows.map((t) => [t.id, t.parentId]))
  const counts = new Map(rows.map((t) => [t.id, 0]))
  for (const leaf of Object.values(itemTopic)) for (let at = leaf; at; at = parent.get(at)) counts.set(at, (counts.get(at) ?? 0) + 1)
  topics = { topics: rows.map((t) => ({ ...t, count: counts.get(t.id) ?? 0 })).filter((t) => t.count > 0), itemTopic }
} catch {
  // An index from before topics existed.
}

const out = resolve(import.meta.dirname, '..', 'frontend', 'public', 'map.local.json')
mkdirSync(resolve(out, '..'), { recursive: true })
writeFileSync(out, JSON.stringify({ items, edges, positions, concepts: [], topics }))
console.log(`Exported ${items.length} items, ${edges.length} links and ${topics.topics.length} topics to ${out}`)
