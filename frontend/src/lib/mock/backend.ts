// In-memory stand-in for the Rust backend, used by `npm run dev:ui`.

import type { Settings } from '../types'

let settings: Settings = {
  libraryPath: '~/Plexus',
  hotkey: 'Ctrl+Alt+V',
  hotkeyAction: 'openCapture',
  startAtLogin: true,
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

const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
  get_settings: () => settings,
  set_settings: (args) => (settings = args.settings as Settings),
  // In the browser the capture window is its own page: open it in a new tab.
  show_capture: () => void window.open('/capture.html', 'plexus-capture'),
  hide_capture: () => void window.close(),
}

export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const handler = commands[command]
  if (!handler) throw new Error(`mock backend: unknown command "${command}"`)
  return handler(args) as T
}
