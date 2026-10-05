// In-memory stand-in for the Rust backend, used by `npm run dev:ui`. Detection here is a rough
// approximation, good enough to exercise the UI; the real rules live in Rust.

import type { CaptureResult, Edge, HotkeyStatus, Item, ItemDetail, ItemLinked, Kind, MapData, Position, Preview, Settings } from '../types'
import { sampleItems } from './samples'

let settings: Settings = {
  libraryPath: '~/Plexus',
  hotkey: 'Ctrl+Alt+V',
  hotkeyAction: 'openCapture',
  startAtLogin: true,
  fetchLinks: true,
}

const items: ItemDetail[] = sampleItems()
const positions = new Map<string, Position>()

/** `?map=map.local.json` loads a real library's map (from `npm run export-map`) instead of the
 * samples. Links then stay as exported; new items are linked by the word-overlap stand-in. */
const mapUrl = new URLSearchParams(location.search).get('map')
let exported: MapData | null = null
async function loadExported(url: string): Promise<MapData> {
  const data: MapData & { items: ItemDetail[] } = await (await fetch(url)).json()
  items.splice(0, items.length, ...data.items)
  data.positions.forEach((p) => positions.set(p.id, p))
  return data
}

// Stand-in for linking by meaning: items sharing longer words are "related".
const words = (item: ItemDetail) => new Set(`${item.title} ${item.body}`.toLowerCase().match(/[a-z]{5,}/g) ?? [])

function linksFor(item: ItemDetail): Edge[] {
  const mine = words(item)
  return items
    .filter((other) => other.id !== item.id)
    .map((other) => {
      const shared = [...words(other)].filter((w) => mine.has(w)).length
      return { src: item.id, dst: other.id, kind: 'similar', weight: Math.min(0.85, 0.55 + 0.08 * shared), reason: null, shared }
    })
    .filter((e) => e.shared > 0)
    .map(({ shared: _, ...edge }) => edge)
}

function allEdges(): Edge[] {
  const seen = new Set<string>()
  return items.flatMap(linksFor).filter((e) => {
    const key = [e.src, e.dst].sort().join('|')
    return !seen.has(key) && seen.add(key)
  })
}

function addItem(title: string, kind: Kind, body: string): CaptureResult {
  const item: ItemDetail = {
    id: crypto.randomUUID(),
    path: `items/mock/${title.toLowerCase().replace(/\W+/g, '-')}.md`,
    kind,
    title,
    sourceUrl: kind === 'url' ? body : null,
    language: null,
    capturedAt: new Date().toISOString(),
    bodyHash: body,
    assetPath: null,
    assetHash: null,
    mime: null,
    size: null,
    status: 'pending',
    extracted: null,
    body,
    assetFile: null,
  }
  const existing = items.find((i) => i.bodyHash === body)
  if (existing) return { status: 'duplicate', id: existing.id, title: existing.title }
  items.push(item)
  emit('item-added', item)
  // Links get their page "read": a nicer title and some text, then they're linked.
  const readDelay = kind === 'url' ? 900 : 0
  if (kind === 'url')
    setTimeout(() => {
      const slug = body.replace(/\/+$/, '').split('/').pop() ?? body
      item.title = decodeURIComponent(slug).replace(/[-_]+/g, ' ').replace(/\.\w+$/, '') || item.title
      item.body = `A page about ${item.title}.`
      item.extracted = 'web@1'
      emit('item-updated', { ...item })
    }, readDelay)
  setTimeout(() => {
    item.status = 'linked'
    const edges = linksFor(item)
    const related = edges.map((e) => ({ id: e.dst, title: items.find((i) => i.id === e.dst)!.title, weight: e.weight }))
    emit('item-linked', { id: item.id, status: 'linked', edges, related } satisfies ItemLinked)
  }, readDelay + 500)
  return { status: 'saved', id: item.id, title, kind }
}

type Handler = (payload: never) => void
const listeners = new Map<string, Set<Handler>>()

export function emit(event: string, payload: unknown) {
  for (const handler of listeners.get(event) ?? []) (handler as (p: unknown) => void)(payload)
}

export function listen<T>(event: string, handler: (payload: T) => void): () => void {
  const set = listeners.get(event) ?? new Set()
  set.add(handler as Handler)
  listeners.set(event, set)
  return () => set.delete(handler as Handler)
}

function preview(text: string): Preview {
  const trimmed = text.trim()
  const firstLine = trimmed.split('\n').find((l) => l.trim()) ?? 'Untitled'
  let kind: Kind = 'text'
  if (/^https?:\/\/\S+$/.test(trimmed)) kind = 'url'
  else if (/[;{}]\s*$|=>|^\s*(fn|def|const|let|import)\b/m.test(trimmed)) kind = 'code'
  const secret = /\b(sk-|ghp_|AKIA)\w{8,}|password\s*[:=]/i.test(trimmed) ? 'an API key' : null
  return { kind, language: null, title: firstLine.replace(/^[#>*\-\s]+/, '').slice(0, 80), secret }
}

const status: HotkeyStatus = {
  backend: 'portal',
  trigger: 'Ctrl+Alt+V',
  error: null,
  systemConfigurable: true,
}

type Command = (args: Record<string, unknown>, headers?: Record<string, string>) => unknown

const commands: Record<string, Command> = {
  get_settings: () => settings,
  set_settings: (args) => (settings = args.settings as Settings),
  hotkey_status: () => status,
  configure_hotkey: () => {},
  // In the browser the capture window is its own page: open it in a new tab.
  show_capture: () => void window.open('/capture.html', 'plexus-capture'),
  hide_capture: () => void window.close(),
  clipboard_image: () => new ArrayBuffer(0),
  pasted_files: () => [],
  capture_preview: (args) => preview(args.text as string),
  capture_text: (args): CaptureResult => {
    const p = preview(args.text as string)
    if (p.secret && !args.force) return { status: 'secret', reason: p.secret }
    return addItem(p.title, p.kind, args.text as string)
  },
  capture_file: (args): CaptureResult => {
    const name = String(args.path).split('/').pop() ?? 'file'
    const note = String(args.note ?? '')
    if (note && preview(note).secret && !args.force) return { status: 'secret', reason: preview(note).secret! }
    const kind = /\.(png|jpe?g|gif|webp)$/i.test(name) ? 'image' : 'file'
    return addItem(note ? preview(note).title : name, kind, note || String(args.path))
  },
  capture_bytes: (_, headers): CaptureResult => {
    const name = decodeURIComponent(headers?.['x-name'] ?? 'file')
    const note = decodeURIComponent(headers?.['x-note'] ?? '')
    if (note && preview(note).secret && headers?.['x-force'] !== '1') return { status: 'secret', reason: preview(note).secret! }
    const kind = headers?.['x-mime']?.startsWith('image/') ? 'image' : 'file'
    return addItem(note ? preview(note).title : name, kind, `${note}\n${name}-${Date.now()}`)
  },
  list_items: (): Item[] => items,
  get_map: async (): Promise<MapData> => {
    if (mapUrl) exported ??= await loadExported(mapUrl)
    const edges = exported?.edges ?? allEdges()
    return { items, edges, positions: [...positions.values()] }
  },
  save_layout: (args) => (args.positions as Position[]).forEach((p) => positions.set(p.id, p)),
  search: (args) => {
    const words = String(args.query).toLowerCase().split(/\W+/).filter(Boolean)
    if (!words.length) return []
    return items
      .filter((i) => words.every((w) => `${i.title} ${i.body}`.toLowerCase().includes(w)))
      .map((i) => i.id)
  },
  open_item_file: () => {},
  open_source: (args) => void window.open(items.find((i) => i.id === args.id)?.sourceUrl ?? '', '_blank'),
  get_item: (args): ItemDetail => {
    const item = items.find((i) => i.id === args.id)
    if (!item) throw new Error(`no item ${args.id}`)
    return item
  },
}

export async function invoke<T>(
  command: string,
  args: Record<string, unknown> | Uint8Array = {},
  headers?: Record<string, string>,
): Promise<T> {
  const handler = commands[command]
  if (!handler) throw new Error(`mock backend: unknown command "${command}"`)
  return handler(args instanceof Uint8Array ? {} : args, headers) as T
}
