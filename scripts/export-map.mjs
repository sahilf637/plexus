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

const out = resolve(import.meta.dirname, '..', 'frontend', 'public', 'map.local.json')
mkdirSync(resolve(out, '..'), { recursive: true })
writeFileSync(out, JSON.stringify({ items, edges, positions }))
console.log(`Exported ${items.length} items and ${edges.length} links to ${out}`)
