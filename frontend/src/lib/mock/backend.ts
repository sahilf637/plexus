// In-memory stand-in for the Rust backend, used by `npm run dev:ui`. Detection here is a rough
// approximation, good enough to exercise the UI; the real rules live in Rust.

import type { UsageReport, Ask, AskCheck, AskFailed, AskStep, AiConnection, AiKeyStatus, ModelInfo, CaptureResult, Concept, ConceptItems, Deleted, Edge, EnrichStatus, HotkeyStatus, Item, ItemDetail, ItemLinked, ItemTopic, Kind, MapData, OlderProgress, Position, Preview, Settings, Tier, Topic, TopicsData } from '../types'
import { sampleConcepts, sampleItems } from './samples'

let settings: Settings = {
  libraryPath: '~/Plexus',
  hotkey: 'Ctrl+Alt+V',
  hotkeyAction: 'openCapture',
  startAtLogin: true,
  fetchLinks: true,
  watchClipboard: false,
  ai: { enabled: false, connections: [], active: '', dailyLimit: 500, askPerDay: 20, askProfile: 'standard', improveOld: false, improvePerDay: 10 },
  // `/?setup` shows the first-run setup.
  setupDone: typeof location === 'undefined' || !new URLSearchParams(location.search).has('setup'),
}

// The keychain, the AI queue and what the AI said (concepts and typed links), simulated.
const keys = new Map<string, string>()

// `/?ai` starts with a provider already connected, to try Ask without the Settings dance.
if (typeof location !== 'undefined' && new URLSearchParams(location.search).has('ai')) {
  settings.ai.enabled = true
  settings.ai.connections = [{ id: 'mock', name: 'Mock AI', preset: 'openai', kind: 'openai', baseUrl: 'https://mock.invalid/v1', model: 'mock', vision: false }]
  settings.ai.active = 'mock'
  keys.set('mock', 'test')
}
const isLocal = (preset: string) => /ollama|lmstudio/.test(preset)
const connectionOf = (id: string) => settings.ai.connections.find((c) => c.id === id)
/** The connection in use can make calls: it has a key, or it's a local server. */
const ready = () => {
  const c = connectionOf(settings.ai.active)
  return !!c && (keys.has(c.id) || isLocal(c.preset))
}
let enrichState: EnrichStatus = { state: 'idle', pending: 0, message: null }
const conceptsByItem = new Map<string, Concept[]>()
const aiEdges: Edge[] = [
  { src: 'sample-1', dst: 'sample-0', kind: 'example_of', weight: 0.84, tier: 'strong', reason: 'The code shows the one-owner rule in action' },
  { src: 'sample-2', dst: 'sample-0', kind: 'extends', weight: 0.8, tier: 'strong', reason: 'The Rust book explains ownership in more depth' },
]
let conceptId = 1
/** Items whose title was set by hand, so the stand-in AI leaves them alone. */
const ownTitles = new Set<string>()
for (const [name, indexes] of Object.entries(sampleConcepts)) {
  const concept = { id: conceptId++, name }
  for (const i of indexes) conceptsByItem.set(`sample-${i}`, [...(conceptsByItem.get(`sample-${i}`) ?? []), concept])
}

const items: ItemDetail[] = sampleItems()
const positions = new Map<string, Position>()
/** Deleted items, by batch, until restored. */
const trash = new Map<string, { items: ItemDetail[] }>()

/** `?map=map.local.json` loads a real library's map (from `npm run export-map`) instead of the
 * samples. Links then stay as exported; new items are linked by the word-overlap stand-in. */
const mapUrl = new URLSearchParams(location.search).get('map')
let exported: MapData | null = null
async function loadExported(url: string): Promise<MapData> {
  const data: MapData & { items: ItemDetail[] } = await (await fetch(url)).json()
  items.splice(0, items.length, ...data.items)
  data.positions.forEach((p) => positions.set(p.id, p))
  if (data.topics?.topics.length) topicsData = data.topics
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
      const weight = Math.min(0.85, 0.55 + 0.08 * shared)
      const tier: Tier = weight >= 0.72 ? 'strong' : weight >= 0.67 ? 'related' : 'maybe'
      const common = [...words(other)].filter((w) => mine.has(w)).slice(0, 3)
      const reason = common.length ? `both mention ${common.join(', ')}` : null
      return { src: item.id, dst: other.id, kind: 'similar', weight, tier, reason, shared }
    })
    .filter((e) => e.shared > 0)
    .map(({ shared: _, ...edge }) => edge)
}

/** Links made by hand: [from, to]. */
const userLinks: [string, string][] = []
const userEdges = (): Edge[] =>
  userLinks
    .filter(([a, b]) => items.some((i) => i.id === a) && items.some((i) => i.id === b))
    .map(([src, dst]) => ({ src, dst, kind: 'user', weight: 1, tier: 'strong', reason: 'linked by you' }))
/** One edge per pair: by hand, then AI (its type and reason), then by meaning. */
function merged(): Edge[] {
  const pairs = new Map<string, Edge>()
  const rank = (e: Edge) => (e.kind === 'user' ? 2 : e.kind === 'similar' ? 0 : 1)
  for (const e of [...(exported?.edges ?? allSimilar()), ...aiEdges.filter(live), ...userEdges()]) {
    const key = [e.src, e.dst].sort().join('|')
    const current = pairs.get(key)
    if (!current || rank(e) > rank(current)) pairs.set(key, current && rank(e) === 1 ? { ...e, weight: Math.max(e.weight, current.weight) } : e)
  }
  return [...pairs.values()].sort((a, b) => b.weight - a.weight)
}
const live = (e: Edge) => items.some((i) => i.id === e.src) && items.some((i) => i.id === e.dst)
const edgesTouching = (...ids: string[]) => merged().filter((e) => ids.includes(e.src) || ids.includes(e.dst))

function allSimilar(): Edge[] {
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
    summary: null,
    titleByAi: false,
    body,
    assetFile: null,
    attachments: [],
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
    const related = edges.map((e) => ({ id: e.dst, title: items.find((i) => i.id === e.dst)!.title, weight: e.weight, tier: e.tier }))
    emit('item-linked', { id: item.id, status: 'linked', edges, related } satisfies ItemLinked)
    joinTopic(item.id, related[0]?.id)
    enrich(item, related[0]?.id)
  }, readDelay + 500)
  return { status: 'saved', id: item.id, title, kind }
}

// Topics: a small tree for the samples (or the exported library's own), kept up to date as
// items are added. New items join the topic of the item they link to, else Loose notes.
let topicsData: TopicsData = sampleTopics()
function sampleTopics(): TopicsData {
  const t = (id: string, parentId: string | null, level: number, label: string, count: number): Topic => ({ id, parentId, level, label, labelBy: 'auto', summary: null, count })
  const itemTopic: Record<string, string> = {}
  const place = (topic: string, ...indexes: number[]) => indexes.forEach((i) => (itemTopic[`sample-${i}`] = topic))
  place('rust', 0, 1, 2)
  place('bread', 3, 4)
  place('memory', 8, 9)
  place('loose', 5, 6, 7)
  return {
    topics: [
      t('code', null, 0, 'Programming', 3),
      t('rust', 'code', 1, 'Rust ownership', 3),
      t('food', null, 0, 'Food', 2),
      t('bread', 'food', 1, 'Sourdough & bread', 2),
      t('learn', null, 0, 'Learning', 2),
      t('memory', 'learn', 1, 'Memory', 2),
      t('loose', null, 0, 'Loose notes', 3),
    ],
    itemTopic,
  }
}
const recount = () => {
  const parent = new Map(topicsData.topics.map((t) => [t.id, t.parentId]))
  const counts = new Map<string, number>()
  for (const leaf of Object.values(topicsData.itemTopic)) for (let at: string | null | undefined = leaf; at; at = parent.get(at)) counts.set(at, (counts.get(at) ?? 0) + 1)
  topicsData = { ...topicsData, topics: topicsData.topics.map((t) => ({ ...t, count: counts.get(t.id) ?? 0 })) }
}
/** A new item joins the topic of its closest link, else Loose notes. */
function joinTopic(id: string, closest?: string) {
  const topic = (closest && topicsData.itemTopic[closest]) || 'loose'
  if (topic === 'loose' && !topicsData.topics.some((t) => t.id === 'loose'))
    topicsData.topics.push({ id: 'loose', parentId: null, level: 0, label: 'Loose notes', labelBy: 'auto', summary: null, count: 0 })
  topicsData.itemTopic[id] = topic
  recount()
  emit('item-topic', { id, topicId: topic } satisfies ItemTopic)
}
let olderDone = 0
let olderTimer: ReturnType<typeof setInterval> | undefined

const conceptIndex = (): ConceptItems[] => {
  const byId = new Map<number, ConceptItems>()
  for (const [item, list] of conceptsByItem)
    for (const c of list) byId.set(c.id, { ...c, items: [...(byId.get(c.id)?.items ?? []), item] })
  return [...byId.values()]
}

const setStatus = (status: EnrichStatus) => emit('enrich-status', (enrichState = status))

/** Stand-in for the AI: a summary, a few concepts from the longer words, a nicer title for notes,
 * and a typed link to the closest item. */
function enrich(item: ItemDetail, closest?: string) {
  if (!settings.ai.enabled) return
  if (!ready()) {
    setStatus({ state: 'paused', pending: 1, message: "The AI provider didn't accept the key (no key set). Check it in Settings." })
    return
  }
  setStatus({ state: 'working', pending: 1, message: null })
  setTimeout(() => {
    const common = [...words(item)].sort((a, b) => b.length - a.length).slice(0, 3)
    const concepts = common.map((name) => {
      const known = [...conceptsByItem.values()].flat().find((c) => c.name === name)
      return known ?? { id: conceptId++, name }
    })
    conceptsByItem.set(item.id, concepts)
    item.summary = item.body.split(/(?<=[.!?])\s/)[0]?.slice(0, 160) || null
    if (item.kind === 'text' && !item.titleByAi && !ownTitles.has(item.id) && common.length) {
      item.title = `${common[0][0].toUpperCase()}${common[0].slice(1)}: ${item.title.slice(0, 40)}`
      item.titleByAi = true
    }
    emit('item-updated', { ...item })
    emit('item-concepts', { id: item.id, concepts })
    if (closest) {
      aiEdges.push({ src: item.id, dst: closest, kind: 'supports', weight: 0.8, tier: 'strong', reason: `Both are about ${common[0] ?? 'the same thing'}` })
      emit('links-changed', { ids: [item.id, closest], edges: edgesTouching(item.id, closest) })
    }
    setStatus({ state: 'idle', pending: 0, message: null })
  }, 1400)
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

// Before setup the shortcut isn't registered yet; `start_hotkey` does it.
const status: HotkeyStatus = settings.setupDone
  ? { backend: 'portal', trigger: 'Ctrl+Alt+V', error: null, systemConfigurable: true }
  : { backend: 'none', trigger: null, error: null, systemConfigurable: false }

type Command = (args: Record<string, unknown>, headers?: Record<string, string>) => unknown

const commands: Record<string, Command> = {
  get_settings: () => settings,
  set_settings: (args) => {
    const previous = settings
    settings = args.settings as Settings
    // A removed connection takes its key with it.
    for (const gone of previous.ai.connections) if (!connectionOf(gone.id)) keys.delete(gone.id)
    if (!settings.ai.enabled) setStatus({ state: 'off', pending: 0, message: null })
    // The slow pass over older notes, sped up: one every few seconds, up to the day's number.
    clearInterval(olderTimer)
    if (settings.ai.enabled && settings.ai.improveOld)
      olderTimer = setInterval(() => {
        if (olderDone < 38 && (settings.ai.improvePerDay === 0 || olderDone < settings.ai.improvePerDay)) olderDone++
      }, 4000)
    return settings
  },
  ai_key_status: (args): AiKeyStatus => ({ hasKey: keys.has(String(args.id)), error: null }),
  set_ai_key: (args) => {
    const key = String(args.key).trim()
    if (key) keys.set(String(args.id), key)
    else keys.delete(String(args.id))
    if (key && enrichState.state === 'paused') setStatus({ state: 'idle', pending: 0, message: null })
  },
  test_ai: async (args) => {
    const badKey = { kind: 'key', message: "The provider didn't accept this key. Check it and try again." }
    await new Promise((done) => setTimeout(done, 700))
    const c = args.connection as AiConnection
    if (!String(args.key ?? '').trim() && !keys.has(c.id) && !isLocal(c.preset)) throw badKey
    if (String(args.key ?? '').includes('bad')) throw badKey
    // A model that is over its limit, and one that answers with nothing usable.
    if (c.model === 'gemma-3-27b-it') throw { kind: 'model', message: 'Something went wrong with this model. Try a different one.' }
    if (c.model === 'o3-mini') throw { kind: 'busy', message: 'The provider is busy, or this model is over its limit right now. Try again shortly, or pick a different model.' }
  },
  list_ai_models: async (args): Promise<ModelInfo[]> => {
    const badKey = { kind: 'key', message: "The provider didn't accept this key. Check it and try again." }
    await new Promise((done) => setTimeout(done, 500))
    const c = args.connection as AiConnection
    if (!String(args.key ?? '').trim() && !keys.has(c.id) && !isLocal(c.preset)) throw badKey
    const lists: Record<string, ModelInfo[]> = {
      openai: [{ id: 'gpt-4.1', vision: true }, { id: 'gpt-4o-mini', vision: true }, { id: 'o3-mini', vision: false }],
      anthropic: [{ id: 'claude-haiku-4-5-20251001', vision: true }, { id: 'claude-sonnet-5-5', vision: true }],
      gemini: [{ id: 'gemini-2.5-flash', vision: true }, { id: 'gemini-2.5-flash-lite-preview-09-2025', vision: true }, { id: 'gemma-3-27b-it', vision: false }],
      groq: [{ id: 'llama-3.3-70b-versatile', vision: false }, { id: 'llama-4-scout', vision: true }],
      ollama: [{ id: 'llama3.2', vision: false }, { id: 'llava:13b', vision: true }],
    }
    return lists[c.preset] ?? []
  },
  enrich_status: (): EnrichStatus => (settings.ai.enabled ? enrichState : { state: 'off', pending: 0, message: null }),
  enrich_retry: () => setStatus({ state: 'idle', pending: 0, message: null }),
  set_title: (args) => {
    const item = items.find((i) => i.id === args.id)
    const title = String(args.title).split(/\s+/).filter(Boolean).join(' ').slice(0, 80)
    if (!item) return
    if (!title) throw new Error('a title needs at least one character')
    // Once set by hand it is never replaced by the AI, even when its answer is on the way.
    ownTitles.add(item.id)
    item.title = title
    item.titleByAi = false
    emit('item-updated', { ...item })
  },
  remove_ai_link: (args) => {
    for (let i = aiEdges.length - 1; i >= 0; i--) {
      const e = aiEdges[i]
      if ([e.src, e.dst].includes(String(args.a)) && [e.src, e.dst].includes(String(args.b))) aiEdges.splice(i, 1)
    }
    emit('links-changed', { ids: [String(args.a), String(args.b)], edges: edgesTouching(String(args.a), String(args.b)) })
  },
  hotkey_status: () => status,
  configure_hotkey: () => {},
  start_hotkey: () =>
    void setTimeout(() => Object.assign(status, { backend: 'portal', trigger: settings.hotkey, error: null, systemConfigurable: true }), 600),
  inspect_folder: (args) => {
    const path = String(args.path ?? settings.libraryPath)
    return { path, notes: path === settings.libraryPath ? items.length : path.includes('Notes') ? 124 : 0, current: path === settings.libraryPath }
  },
  change_library: (args) => {
    if (args.path === settings.libraryPath) throw new Error('That is the library you have open already.')
    settings.libraryPath = String(args.path)
  },
  pick_folder: () => window.prompt('Folder', '~/Documents/Notes') || null,
  restart_app: () => location.reload(),
  // In the browser the capture window is its own page: open it in a new tab.
  show_capture: () => void window.open('/capture.html', 'plexus-capture'),
  hide_capture: () => void window.close(),
  clipboard_image: () => new ArrayBuffer(0),
  pasted_files: () => [],
  clipboard_files: () => [],
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
  capture_many: (_, headers): CaptureResult => {
    const parts: { name?: string; path?: string; mime?: string }[] = JSON.parse(decodeURIComponent(headers?.['x-parts'] ?? '[]'))
    const note = decodeURIComponent(headers?.['x-note'] ?? '')
    if (preview(note).secret && headers?.['x-force'] !== '1') return { status: 'secret', reason: preview(note).secret! }
    const first = parts[0] ?? {}
    const kind = first.mime?.startsWith('image/') || /\.(png|jpe?g|gif|webp)$/i.test(first.path ?? '') ? 'image' : 'file'
    return addItem(preview(note).title, kind, `${note}\n${parts.length} files ${Date.now()}`)
  },
  list_items: (): Item[] => items,
  get_map: async (): Promise<MapData> => {
    if (mapUrl) exported ??= await loadExported(mapUrl)
    return { items, edges: merged(), positions: [...positions.values()], concepts: conceptIndex(), topics: topicsData }
  },
  save_layout: (args) => (args.positions as Position[]).forEach((p) => positions.set(p.id, p)),
  search: (args) => {
    const words = String(args.query).toLowerCase().split(/\W+/).filter(Boolean)
    if (!words.length) return []
    return items
      .filter((i) => words.every((w) => `${i.title} ${i.body}`.toLowerCase().includes(w)))
      .map((i) => i.id)
  },
  link_items: (args) => {
    const [from, to] = [String(args.from), String(args.to)]
    if (from !== to && !userLinks.some(([a, b]) => a === from && b === to)) userLinks.push([from, to])
    emit('links-changed', { ids: [from, to], edges: edgesTouching(from, to) })
  },
  unlink_items: (args) => {
    const [a, b] = [String(args.a), String(args.b)]
    for (let i = userLinks.length - 1; i >= 0; i--) {
      const [x, y] = userLinks[i]
      if ((x === a && y === b) || (x === b && y === a)) userLinks.splice(i, 1)
    }
    emit('links-changed', { ids: [a, b], edges: edgesTouching(a, b) })
  },
  delete_items: (args): Deleted => {
    const ids = (args.ids as string[]).filter((id) => items.some((i) => i.id === id))
    const batch = crypto.randomUUID().replace(/-/g, '').toUpperCase()
    trash.set(batch, { items: items.filter((i) => ids.includes(i.id)) })
    for (const id of ids) items.splice(items.findIndex((i) => i.id === id), 1)
    if (ids.length) {
      for (const id of ids) delete topicsData.itemTopic[id]
      recount()
      emit('items-deleted', ids)
    }
    return { batch, ids }
  },
  restore_items: (args): number => {
    const batch = trash.get(args.batch as string)
    if (!batch) throw new Error('this was deleted too long ago to restore')
    trash.delete(args.batch as string)
    for (const item of batch.items) {
      items.push(item)
      emit('item-added', item)
      setTimeout(() => emit('item-linked', { id: item.id, status: 'linked', edges: linksFor(item), related: [] } satisfies ItemLinked), 300)
    }
    return batch.items.length
  },
  get_topics: (): TopicsData => topicsData,
  rename_topic: (args) => {
    const name = String(args.name).trim()
    if (!name || name.length > 60) throw new Error('a topic name needs 1 to 60 characters')
    topicsData = { ...topicsData, topics: topicsData.topics.map((t) => (t.id === args.id ? { ...t, label: name, labelBy: 'you' } : t)) }
    emit('topics-changed', topicsData)
  },
  reset_topic_name: (args) => {
    topicsData = { ...topicsData, topics: topicsData.topics.map((t) => (t.id === args.id ? { ...t, labelBy: 'auto' } : t)) }
    emit('topics-changed', topicsData)
  },
  regroup_topics: () => emit('topics-changed', topicsData),
  ask_start: (args): Ask => {
    if (!settings.ai.connections.length) throw new Error('Connect an AI in Settings to get written answers.')
    const id = `ask${asks.length + 1}-${Date.now()}`
    const follows = args.thread ? asks.find((a) => a.threadId === args.thread || a.id === args.thread) : undefined
    if (args.thread && !follows) throw new Error('That conversation is gone; start a new question.')
    const ask: Ask = {
      id,
      threadId: follows?.threadId ?? id,
      scope: args.scope ? String(args.scope) : null,
      question: String(args.question).trim(),
      answer: '',
      effort: 'explain',
      state: 'running',
      unsupported: false,
      limitHit: null,
      createdAt: new Date().toISOString(),
      sources: [],
    }
    if (!ask.question) throw new Error('Ask something first.')
    asks.unshift(ask)
    stopped.delete(ask.id)
    void playAsk(ask)
    return { ...ask }
  },
  ask_check: async (args): Promise<AskCheck> => {
    await new Promise((done) => setTimeout(done, 1200))
    const c = args.connection as AiConnection
    if (!String(args.key ?? '').trim() && !keys.has(c.id) && !isLocal(c.preset)) throw new Error("the provider didn't accept the key: no key set")
    // Local servers often can't call tools; the others can.
    return isLocal(c.preset)
      ? { ok: true, protocol: 'json', message: "Works, using simpler JSON steps (this model can't call tools). Answers may be a little less reliable." }
      : { ok: true, protocol: 'tools', message: 'Works: this model calls tools and cited the right note.' }
  },
  ask_continue: () => {
    throw new Error("That question can't be continued any more; ask it again.")
  },
  ai_usage: (): UsageReport => {
    const day = (back: number) => new Date(Date.now() - back * 86_400_000).toISOString().slice(0, 10)
    const asked = asks.length
    const notes = settings.ai.enabled ? 38 + olderDone : 0
    const today = (calls: number, failed = 0, limited = 0) => ({ calls, failed, limited, tokensIn: calls * 900, tokensOut: calls * 140 })
    const byPurpose = [
      { purpose: 'notes', ...today(notes, 1) },
      { purpose: 'ask', ...today(asked * 4, 0, asked > 2 ? 2 : 0) },
      { purpose: 'topics', ...today(settings.ai.enabled ? 6 : 0) },
    ].filter((p) => p.calls > 0)
    const calls = byPurpose.reduce((n, p) => n + p.calls, 0)
    const counted = byPurpose.filter((p) => p.purpose !== 'ask').reduce((n, p) => n + p.calls, 0)
    // Two weeks of made-up history: [notes, questions, topics] per day, oldest first.
    const earlier: [number, number, number][] = [[0, 0, 0], [4, 0, 0], [22, 8, 2], [9, 14, 0], [0, 0, 0], [0, 6, 0], [31, 22, 5], [12, 0, 3], [60, 41, 8], [88, 52, 12], [24, 18, 2], [40, 9, 4], [18, 31, 1]]
    const row = (d: number, [n, q, t]: [number, number, number]) => {
      const by = [
        { purpose: 'notes', calls: n, tokens: n * 1040 },
        { purpose: 'ask', calls: q, tokens: q * 2400 },
        { purpose: 'topics', calls: t, tokens: t * 900 },
      ].filter((p) => p.calls > 0)
      return { day: day(d), calls: n + q + t, tokens: by.reduce((x, p) => x + p.tokens, 0), by }
    }
    const todayBy = byPurpose.map((p) => ({ purpose: p.purpose, calls: p.calls, tokens: p.tokensIn + p.tokensOut }))
    return {
      today: { calls, failed: byPurpose.reduce((n, p) => n + p.failed, 0), limited: byPurpose.reduce((n, p) => n + p.limited, 0), tokensIn: calls * 900, tokensOut: calls * 140 },
      days: [...earlier.map((c, i) => row(13 - i, c)), { day: day(0), calls, tokens: todayBy.reduce((x, p) => x + p.tokens, 0), by: todayBy }],
      byPurpose,
      countedToday: counted,
      asksToday: asked,
    }
  },
  ask_stop: (args) => void stopped.add(String(args.id)),
  ask_resume: () => {
    throw new Error("That question can't be continued; ask it again.")
  },
  ask_history: (): Ask[] => asks.map((a) => ({ ...a })),
  ask_delete: (args) => {
    const at = asks.findIndex((a) => a.id === args.id)
    if (at >= 0) asks.splice(at, 1)
  },
  older_status: (): OlderProgress => ({ done: olderDone, total: 38, today: Math.min(olderDone, settings.ai.improvePerDay) }),
  open_item_file: () => {},
  open_url: (args) => void window.open(String(args.url), '_blank'),
  open_source: (args) => void window.open(items.find((i) => i.id === args.id)?.sourceUrl ?? '', '_blank'),
  get_item: (args): ItemDetail => {
    const item = items.find((i) => i.id === args.id)
    if (!item) throw new Error(`no item ${args.id}`)
    return item
  },
}

const asks: Ask[] = []
const stopped = new Set<string>()

/** Stand-in for the agent: looks for the question's words, "reads" the best note and answers with
 * a sentence from it, one step at a time so the UI shows the same events as the real thing. */
async function playAsk(ask: Ask) {
  const pause = (ms: number) => new Promise((done) => setTimeout(done, ms))
  const words = ask.question.toLowerCase().split(/\W+/).filter((w) => w.length > 3)
  const scored = items
    .map((i) => ({ item: i, score: words.filter((w) => `${i.title} ${i.body}`.toLowerCase().includes(w)).length }))
    .filter((s) => s.score > 0)
    .sort((a, b) => b.score - a.score)
    .map((s) => s.item)
  // A follow-up that names nothing new goes on with the note the conversation cited.
  const before = asks.find((a) => a.threadId === ask.threadId && a.id !== ask.id && a.sources.length)
  const earlierNote = before ? items.find((i) => i.id === before.sources[0].itemId) : undefined
  if (!scored.length && earlierNote) scored.push(earlierNote)
  const step = (n: number, tool: AskStep['tool'], label: string, ids: string[]) =>
    emit('ask-step', { askId: ask.id, n, tool, label, items: ids } satisfies AskStep)
  const finish = (patch: Partial<Ask>) => {
    Object.assign(ask, patch)
    emit('ask-done', { ...ask })
  }
  await pause(500)
  if (stopped.has(ask.id)) return finish({ state: 'stopped', answer: 'Stopped before anything was found.' })
  step(1, 'search', `searched for "${words.slice(0, 3).join(' ') || ask.question}"`, scored.slice(0, 5).map((i) => i.id))
  if (!scored.length) {
    await pause(500)
    return finish({ state: 'done', effort: 'lookup', answer: "Your notes don't cover this." })
  }
  await pause(700)
  if (stopped.has(ask.id)) return finish({ state: 'stopped', answer: `So far I found "${scored[0].title}".` })
  step(2, 'read', `read ${scored[0].title}`, [scored[0].id])
  await pause(800)
  if (stopped.has(ask.id)) return finish({ state: 'stopped', answer: `So far I found "${scored[0].title}".` })
  // The sentence (from the note or the text read out of its file) sharing the most words.
  const text = `${scored[0].body}\n${scored[0].extractedText ?? ''}`.replace(/<!-- page \d+ -->/g, '')
  const sentences = text.split(/(?<=[.!?])\s|\n+/).map((t) => t.replace(/[#>*_|]/g, '').trim()).filter((t) => t.length > 12)
  const overlap = (t: string) => words.filter((w) => t.toLowerCase().includes(w)).length
  const sentence = sentences.sort((a, b) => overlap(b) - overlap(a))[0] ?? scored[0].title
  finish({
    state: 'done',
    effort: 'lookup',
    answer: `**${scored[0].title}**: ${sentence} [1]\n\n- Read in your notes: ${scored[0].title} [1]`,
    limitHit: 'steps',
    sources: [{ n: 1, itemId: scored[0].id, quote: sentence.slice(0, 60) }],
  })
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
