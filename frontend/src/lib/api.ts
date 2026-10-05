// The only module that talks to the Rust side. Outside Tauri (`npm run dev:ui`) every call
// goes to an in-memory mock instead, so the UI can be built in a plain browser.

import type { AiKeyStatus, CaptureResult, Deleted, EnrichStatus, HotkeyStatus, ModelInfo, Item, ItemDetail, MapData, Position, Preview, Settings } from './types'
import * as mock from './mock/backend'

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function call<T>(
  command: string,
  args?: Record<string, unknown> | Uint8Array,
  headers?: Record<string, string>,
): Promise<T> {
  if (!inTauri) return mock.invoke<T>(command, args, headers)
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args, headers ? { headers } : undefined)
}

/** Subscribe to a backend event. Resolves to an unsubscribe function. */
export async function on<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return mock.listen(event, handler)
  const { listen } = await import('@tauri-apps/api/event')
  return listen<T>(event, (e) => handler(e.payload))
}

export const getSettings = () => call<Settings>('get_settings')
export const setSettings = (settings: Settings) => call<Settings>('set_settings', { settings })
export const aiKeyStatus = () => call<AiKeyStatus>('ai_key_status')
/** Stores the key for the selected provider in the system keychain (empty removes it). */
export const setAiKey = (key: string) => call<void>('set_ai_key', { key })
/** A tiny request with the saved settings and key; rejects with why it failed. */
export const testAi = () => call<void>('test_ai')
/** The chat models the selected provider offers (needs the saved key, unless it's a local server). */
export const listAiModels = () => call<ModelInfo[]>('list_ai_models')
export const enrichStatus = () => call<EnrichStatus>('enrich_status')
/** Tries again after a pause (a wrong key fixed, a limit raised). */
export const enrichRetry = () => call<void>('enrich_retry')
/** Puts back the title an item had before the AI renamed it. */
export const revertTitle = (id: string) => call<void>('revert_title', { id })
/** Removes a link the AI proposed; it won't come back. */
export const removeAiLink = (a: string, b: string) => call<void>('remove_ai_link', { a, b })
export const hotkeyStatus = () => call<HotkeyStatus>('hotkey_status')
export const configureHotkey = () => call<void>('configure_hotkey')

export const showCapture = () => call<void>('show_capture')
export const hideCapture = () => call<void>('hide_capture')

/** The system clipboard's image as PNG, or null. Needed because the webview's paste event
 * doesn't carry images on Linux. */
export async function clipboardImage(): Promise<Blob | null> {
  const bytes = await call<ArrayBuffer | number[]>('clipboard_image')
  const data = new Uint8Array(bytes)
  return data.byteLength ? new Blob([data], { type: 'image/png' }) : null
}

/** Pasted text that is only paths to existing files (copied in a file manager) → those paths. */
export const pastedFiles = (text: string) => call<string[]>('pasted_files', { text })
/** Files copied in a file manager, read from the clipboard by the backend: on Linux the paste
 * event for them is empty. */
export const clipboardFiles = () => call<string[]>('clipboard_files')

export const capturePreview = (text: string) => call<Preview>('capture_preview', { text })
/** `html` is the same copy as rich text, when the source had it (kept as markdown if structured). */
export const captureText = (text: string, force = false, html?: string) =>
  call<CaptureResult>('capture_text', { text, force, html })
/** A file by path, with an optional note about it (`force` keeps a note that looks like a secret). */
export const captureFile = (path: string, note = '', force = false) =>
  call<CaptureResult>('capture_file', { path, note, force })

export const listItems = () => call<Item[]>('list_items')
export const getItem = (id: string) => call<ItemDetail>('get_item', { id })
export const getMap = () => call<MapData>('get_map')
export const saveLayout = (positions: Position[]) => call<void>('save_layout', { positions })
export const search = (query: string) => call<string[]>('search', { query })
/** Opens the item's original (or its note); `attachment` picks one of the extra originals. */
export const openItemFile = (id: string, attachment?: number) => call<void>('open_item_file', { id, attachment })
export const openSource = (id: string) => call<void>('open_source', { id })
/** Links two items by hand (kept in the note of `from`); such links are always strong. */
export const linkItems = (from: string, to: string) => call<void>('link_items', { from, to })
/** Removes a link made by hand. */
export const unlinkItems = (a: string, b: string) => call<void>('unlink_items', { a, b })
/** Moves items (and their files) to the library's trash. Undo with `restoreItems(batch)`. */
export const deleteItems = (ids: string[]) => call<Deleted>('delete_items', { ids })
/** Puts a deleted batch back; the items are linked again. Resolves to how many came back. */
export const restoreItems = (batch: string) => call<number>('restore_items', { batch })

/** A URL the webview can load a library file from (images in the item panel). */
export async function fileUrl(path: string): Promise<string> {
  if (!inTauri) return path
  const { convertFileSrc } = await import('@tauri-apps/api/core')
  return convertFileSrc(path)
}

/** Pasted images and files go over as raw bytes, not JSON; the name and note ride in headers. */
export async function captureBytes(blob: Blob, name: string, note = '', force = false): Promise<CaptureResult> {
  const bytes = new Uint8Array(await blob.arrayBuffer())
  return call<CaptureResult>('capture_bytes', bytes, {
    'x-name': encodeURIComponent(name),
    'x-mime': blob.type,
    'x-note': encodeURIComponent(note),
    'x-force': force ? '1' : '0',
  })
}

/** Several images and files with one note, saved as one item. Pasted ones go as raw bytes,
 * one after another; dropped ones by path. */
export async function captureMany(
  parts: ({ blob: Blob; name: string } | { path: string })[],
  note: string,
  force = false,
): Promise<CaptureResult> {
  const blobs = parts.flatMap((p) => ('blob' in p ? [p.blob] : []))
  const bytes = new Uint8Array(await new Blob(blobs).arrayBuffer())
  const described = parts.map((p) =>
    'blob' in p ? { name: p.name, mime: p.blob.type, size: p.blob.size } : { path: p.path },
  )
  return call<CaptureResult>('capture_many', bytes, {
    'x-parts': encodeURIComponent(JSON.stringify(described)),
    'x-note': encodeURIComponent(note),
    'x-force': force ? '1' : '0',
  })
}

/** Files dropped on the window from the file manager (Tauri gives us their paths). */
export async function onFileDrop(handlers: {
  over: (over: boolean) => void
  drop: (paths: string[]) => void
}): Promise<() => void> {
  if (!inTauri) return () => {}
  const { getCurrentWebview } = await import('@tauri-apps/api/webview')
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'enter' || payload.type === 'over') handlers.over(true)
    else if (payload.type === 'leave') handlers.over(false)
    else if (payload.type === 'drop') {
      handlers.over(false)
      handlers.drop(payload.paths)
    }
  })
}
