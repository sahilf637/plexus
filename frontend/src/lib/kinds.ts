import type { IconName } from '../components/Icon.svelte'
import type { Kind, ProviderKind, Tier } from './types'

/** How each kind is named in the UI. */
export const kindLabel: Record<Kind, string> = {
  text: 'Text',
  code: 'Code',
  url: 'Link',
  image: 'Image',
  file: 'File',
}

/** The icon that stands for each kind. */
export const kindIcon: Record<Kind, IconName> = {
  text: 'text',
  code: 'code',
  url: 'globe',
  image: 'image',
  file: 'file',
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

/** Link types the AI can give; everything else is "similar" (by meaning) or "user" (by hand). */
const aiLinkLabels: Record<string, [outgoing: string, incoming: string]> = {
  supports: ['supports', 'supported by'],
  contradicts: ['contradicts', 'contradicted by'],
  extends: ['extends', 'extended by'],
  example_of: ['is an example of', 'has this example'],
  related: ['', ''],
}

export const isAiLink = (kind: string) => kind in aiLinkLabels

/** How a link reads from one of its ends ("extends" on the item that does the extending, "extended
 * by" on the other). Empty for plain links. */
export function linkLabel(kind: string, outgoing: boolean): string {
  return aiLinkLabels[kind]?.[outgoing ? 0 : 1] ?? ''
}

/** Providers offered in Settings. Everything but Anthropic and Gemini speaks the OpenAI chat API. */
export interface AiPreset {
  id: string
  label: string
  kind: ProviderKind
  baseUrl: string
  model: string
  /** Runs on this computer, so no key is needed. */
  local?: boolean
  /** Whether the default model reads images. */
  vision: boolean
}

export const aiPresets: AiPreset[] = [
  { id: 'openai', label: 'OpenAI', kind: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', vision: true },
  { id: 'anthropic', label: 'Anthropic', kind: 'anthropic', baseUrl: 'https://api.anthropic.com', model: 'claude-haiku-4-5-20251001', vision: true },
  { id: 'gemini', label: 'Google Gemini', kind: 'gemini', baseUrl: 'https://generativelanguage.googleapis.com', model: 'gemini-2.5-flash', vision: true },
  { id: 'groq', label: 'Groq', kind: 'openai', baseUrl: 'https://api.groq.com/openai/v1', model: 'llama-3.3-70b-versatile', vision: false },
  { id: 'openrouter', label: 'OpenRouter', kind: 'openai', baseUrl: 'https://openrouter.ai/api/v1', model: 'openai/gpt-4o-mini', vision: true },
  { id: 'ollama', label: 'Ollama (this computer)', kind: 'openai', baseUrl: 'http://localhost:11434/v1', model: 'llama3.2', local: true, vision: false },
  { id: 'lmstudio', label: 'LM Studio (this computer)', kind: 'openai', baseUrl: 'http://localhost:1234/v1', model: '', local: true, vision: false },
]

