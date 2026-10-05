// In-memory stand-in for the Rust backend, used by `npm run dev:ui`. Detection here is a rough
// approximation, good enough to exercise the UI; the real rules live in Rust.

import type { CaptureResult, HotkeyStatus, Item, ItemDetail, Kind, Preview, Settings } from '../types'

let settings: Settings = {
  libraryPath: '~/Plexus',
  hotkey: 'Ctrl+Alt+V',
  hotkeyAction: 'openCapture',
  startAtLogin: true,
}

const items: ItemDetail[] = []

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
    body,
    assetFile: null,
  }
  const existing = items.find((i) => i.bodyHash === body)
  if (existing) return { status: 'duplicate', id: existing.id, title: existing.title }
  items.push(item)
  emit('item-added', item)
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
    return addItem(name, 'file', String(args.path))
  },
  capture_bytes: (_, headers): CaptureResult => {
    const name = decodeURIComponent(headers?.['x-name'] ?? 'file')
    const kind = headers?.['x-mime']?.startsWith('image/') ? 'image' : 'file'
    return addItem(name, kind, `${name}-${Date.now()}`)
  },
  list_items: (): Item[] => items,
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
