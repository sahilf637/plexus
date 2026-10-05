// The only module that talks to the Rust side. Outside Tauri (`npm run dev:ui`) every call
// goes to an in-memory mock instead, so the UI can be built in a plain browser.

import type { Settings } from './types'
import * as mock from './mock/backend'

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mock.invoke<T>(command, args)
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args)
}

/** Subscribe to a backend event. Resolves to an unsubscribe function. */
export async function on<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return mock.listen(event, handler)
  const { listen } = await import('@tauri-apps/api/event')
  return listen<T>(event, (e) => handler(e.payload))
}

export const getSettings = () => call<Settings>('get_settings')
export const setSettings = (settings: Settings) => call<Settings>('set_settings', { settings })
export const showCapture = () => call<void>('show_capture')
export const hideCapture = () => call<void>('hide_capture')
