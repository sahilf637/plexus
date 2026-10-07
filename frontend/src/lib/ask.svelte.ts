// The Ask feature's state, shared by the panel (what to show) and the map (where the agent is
// looking). Events from the backend arrive through the `on...` functions.

import { SvelteSet } from 'svelte/reactivity'
import { askContinue, askDelete, askHistory, askResume, askStart, askStop, search } from './api'
import { toast } from './toasts.svelte'
import type { Ask, AskFailed, AskStep } from './types'

/** Where the question was last put: the whole library, or one topic or note. */
export interface AskScope {
  id: string
  label: string
  kind: 'note' | 'topic'
}

/** Turns a stored scope id back into what the panel shows for it (set by the app, which knows the
 * notes and topics). Null when it is gone. */
export const scopeResolver: { fn: ((id: string) => AskScope | null) | null } = { fn: null }

interface AskState {
  open: boolean
  historyOpen: boolean
  scope: AskScope | null
  /** The question on screen: running, or finished (also a past one reopened from the history). */
  current: Ask | null
  /** The questions before `current` in the same conversation, oldest first. */
  earlier: Ask[]
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
  earlier: [],
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
  if (/limiting requests/i.test(message)) return 'The AI provider is limiting requests (often a per-minute cap). Wait a little, then press Continue.'
  return message
}

export async function loadHistory() {
  try {
    ask.history = await askHistory()
  } catch {}
}

/** A conversation can go on once its latest question has an answer. */
const canFollow = (a: Ask | null): a is Ask => !!a && !!a.answer && ['done', 'partial', 'stopped'].includes(a.state)

/** Asks a question, as a follow-up when there is an answer on screen to follow. With no AI
 * connected, shows what search finds instead. */
export async function startAsk(question: string) {
  const text = question.trim()
  if (!text || running()) return
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
  const follows = canFollow(ask.current) ? ask.current : null
  try {
    // An empty scope says "everything": without it a follow-up would stay with the conversation's.
    const started = await askStart(text, ask.scope?.id ?? '', follows?.threadId)
    ask.earlier = follows ? [...ask.earlier, follows] : []
    ask.steps = []
    ask.current = started
  } catch (e) {
    const message = String(e)
    if (message.includes('Connect an AI')) {
      ask.current = null
      ask.earlier = []
      ask.fallback = { question: text, ids: (await search(text).catch(() => [])).slice(0, 8) }
    } else {
      ask.error = plainError(message)
    }
  }
}

/** An answer that used up its steps can go on looking, from where it stopped. */
export async function keepLooking() {
  if (!ask.current || !ask.current.limitHit) return
  ask.error = null
  try {
    await askContinue(ask.current.id)
    ask.current = { ...ask.current, state: 'running' }
  } catch (e) {
    ask.error = plainError(String(e))
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
  ask.earlier = []
  ask.steps = []
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
}

/** The conversations in the history, the most recently used first. */
/** Conversations deleted a moment ago, still undoable: hidden now, removed for good after a while. */
const pendingDelete = new SvelteSet<string>()
const UNDO_MS = 8000

export function threads(): { id: string; first: Ask; latest: Ask; count: number }[] {
  const byThread = new Map<string, Ask[]>()
  for (const past of ask.history.filter((a) => !pendingDelete.has(a.threadId))) byThread.set(past.threadId, [...(byThread.get(past.threadId) ?? []), past])
  return [...byThread.entries()]
    .map(([id, asks]) => {
      const sorted = asks.toSorted((a, b) => a.createdAt.localeCompare(b.createdAt))
      return { id, first: sorted[0], latest: sorted[sorted.length - 1], count: sorted.length }
    })
    .sort((a, b) => b.latest.createdAt.localeCompare(a.latest.createdAt))
}

/** Shows a past conversation, ready to go on with. */
export function reopen(threadId: string) {
  const asks = ask.history.filter((a) => a.threadId === threadId).toSorted((a, b) => a.createdAt.localeCompare(b.createdAt))
  if (!asks.length) return
  const latest = asks[asks.length - 1]
  ask.current = latest
  ask.scope = latest.scope ? (scopeResolver.fn?.(latest.scope) ?? null) : null
  ask.earlier = asks.slice(0, -1)
  ask.steps = []
  ask.error = null
  ask.canResume = false
  ask.fallback = null
  ask.historyOpen = false
}

/** Deletes a whole conversation, with a moment to change your mind. */
export function removeThread(threadId: string) {
  const ids = ask.history.filter((a) => a.threadId === threadId).map((a) => a.id)
  if (!ids.length) return
  pendingDelete.add(threadId)
  if (ask.current?.threadId === threadId) {
    // The open conversation goes, but the list you are deleting from stays.
    const open = ask.historyOpen
    newAsk()
    ask.historyOpen = open
  }
  const timer = setTimeout(async () => {
    await Promise.all(ids.map((id) => askDelete(id).catch(() => {})))
    ask.history = ask.history.filter((a) => a.threadId !== threadId)
    pendingDelete.delete(threadId)
  }, UNDO_MS)
  toast('Deleted the conversation', { label: 'Undo', run: () => { clearTimeout(timer); pendingDelete.delete(threadId) } }, UNDO_MS)
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
