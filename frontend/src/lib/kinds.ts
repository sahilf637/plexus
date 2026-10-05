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

/** How strong a link looks, 0–1. Raw similarities from the embedding model sit in a narrow band
 * (links start at 0.60, and few pass 0.88), so they are stretched for display. */
export function linkStrength(weight: number): number {
  return Math.min(1, Math.max(0.08, (weight - 0.58) / 0.3))
}
