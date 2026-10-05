// The only module that talks to the Rust side. Outside Tauri (`npm run dev:ui`) every call
// goes to an in-memory mock instead, so the UI can be built in a plain browser.

import type { CaptureResult, HotkeyStatus, Item, ItemDetail, MapData, Position, Preview, Settings } from './types'
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

export const capturePreview = (text: string) => call<Preview>('capture_preview', { text })
export const captureText = (text: string, force = false) =>
  call<CaptureResult>('capture_text', { text, force })
/** A file by path, with an optional note about it (`force` keeps a note that looks like a secret). */
export const captureFile = (path: string, note = '', force = false) =>
  call<CaptureResult>('capture_file', { path, note, force })

export const listItems = () => call<Item[]>('list_items')
export const getItem = (id: string) => call<ItemDetail>('get_item', { id })
export const getMap = () => call<MapData>('get_map')
export const saveLayout = (positions: Position[]) => call<void>('save_layout', { positions })
export const search = (query: string) => call<string[]>('search', { query })
export const openItemFile = (id: string) => call<void>('open_item_file', { id })
export const openSource = (id: string) => call<void>('open_source', { id })

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
