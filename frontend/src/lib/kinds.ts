import type { Kind } from './types'

/** How each kind is named in the UI. */
export const kindLabel: Record<Kind, string> = {
  text: 'Text',
  code: 'Code',
  url: 'Link',
  image: 'Image',
  file: 'File',
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
