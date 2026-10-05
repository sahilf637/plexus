import type { Kind, Tier } from './types'

/** How each kind is named in the UI. */
export const kindLabel: Record<Kind, string> = {
  text: 'Text',
  code: 'Code',
  url: 'Link',
  image: 'Image',
  file: 'File',
}

/** Tiers from most to least sure. */
export const tiers: Tier[] = ['strong', 'related', 'maybe']

export const tierLabel: Record<Tier, string> = {
  strong: 'Strongly related',
  related: 'Related',
  maybe: 'Maybe related',
}

/** Whether a link of tier `tier` shows when the map shows links down to `lowest`. */
export function tierShown(tier: Tier, lowest: Tier): boolean {
  return tiers.indexOf(tier) <= tiers.indexOf(lowest)
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
