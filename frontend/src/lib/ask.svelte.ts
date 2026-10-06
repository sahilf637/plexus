// The Ask feature's state, shared by the panel (what to show) and the map (where the agent is
// looking). Events from the backend arrive through the `on...` functions.

import { askDelete, askHistory, askResume, askStart, askStop, search } from './api'
import type { Ask, AskFailed, AskStep } from './types'

/** Where the question was last put: the whole library, or one topic or note. */
export interface AskScope {
  id: string
  label: string
}

interface AskState {
  open: boolean
  historyOpen: boolean
  scope: AskScope | null
  /** The question on screen: running, or finished (also a past one reopened from the history). */
  current: Ask | null
  /** The tools it has run so far, oldest first. */
  steps: AskStep[]
  history: Ask[]
  /** Why the last question didn't run or didn't finish. */
  error: string | null
  canResume: boolean
  /** The map follows the agent: it eases to the latest note when that is out of sight. */
  follow: boolean
  /** With no AI connected: the notes search found for the question instead. */
  fallback: { question: string; ids: string[] } | null
}

export const ask = $state<AskState>({
  open: false,
  historyOpen: false,
  scope: null,
  current: null,
  steps: [],
  history: [],
  error: null,
  canResume: false,
  follow: true,
  fallback: null,
})

export const running = () => ask.current?.state === 'running'

/** Notes the agent has looked at, in the order it first did. */
export function trail(): string[] {
  const seen: string[] = []
  for (const step of ask.steps) for (const id of step.items) if (!seen.includes(id)) seen.push(id)
  return seen
}

/** Notes the current answer cites. */
export const cited = (): string[] => ask.current?.sources.map((s) => s.itemId) ?? []

function plainError(message: string): string {
  if (/daily/i.test(message)) return "You've used today's questions. Raise the limit in Settings → AI, or try tomorrow."
  return message
}

export async function loadHistory() {
  try {
    ask.history = await askHistory()
  } catch {}
}

/** Asks a question. With no AI connected, shows what search finds instead. */
export async function startAsk(question: string) {
  const text = question.trim()
  if (!text || running()) return
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
  try {
    ask.steps = []
    ask.current = await askStart(text, ask.scope?.id)
  } catch (e) {
    ask.current = null
    const message = String(e)
    if (message.includes('Connect an AI')) {
      ask.fallback = { question: text, ids: (await search(text).catch(() => [])).slice(0, 8) }
    } else {
      ask.error = plainError(message)
    }
  }
}

export function stopAsk() {
  if (ask.current) askStop(ask.current.id)
}

export async function resumeAsk() {
  if (!ask.current) return
  ask.error = null
  ask.canResume = false
  try {
    await askResume(ask.current.id)
    ask.current = { ...ask.current, state: 'running' }
  } catch (e) {
    ask.error = plainError(String(e))
  }
}

/** Goes back to an empty panel, ready for a new question. */
export function newAsk() {
  ask.current = null
  ask.steps = []
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
}

/** Shows a past question and its answer. */
export function reopen(past: Ask) {
  ask.current = past
  ask.steps = []
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
}

export async function removeAsk(id: string) {
  await askDelete(id)
  ask.history = ask.history.filter((a) => a.id !== id)
  if (ask.current?.id === id) newAsk()
}

export function onAskStep(step: AskStep) {
  if (step.askId === ask.current?.id) ask.steps = [...ask.steps, step]
}

export function onAskDone(done: Ask) {
  if (done.id === ask.current?.id) ask.current = done
  ask.history = [done, ...ask.history.filter((a) => a.id !== done.id)]
}

export function onAskFailed(failed: AskFailed) {
  if (failed.askId !== ask.current?.id) return
  ask.current = { ...ask.current, state: 'failed' }
  ask.error = plainError(failed.message)
  ask.canResume = failed.canResume
}
