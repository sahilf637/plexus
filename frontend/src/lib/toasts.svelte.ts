// Short messages in the corner of the map window, optionally with one action.

export interface Toast {
  id: number
  message: string
  action?: { label: string; run: () => void }
}

const TIMEOUT_MS = 4500
let nextId = 1

export const toasts = $state<Toast[]>([])

export function toast(message: string, action?: Toast['action'], ms = TIMEOUT_MS) {
  const id = nextId++
  toasts.push({ id, message, action })
  setTimeout(() => dismiss(id), ms)
}

export function dismiss(id: number) {
  const index = toasts.findIndex((t) => t.id === id)
  if (index >= 0) toasts.splice(index, 1)
}
