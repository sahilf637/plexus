// Finding a quoted passage (an Ask citation) in a note as it is shown, and marking it.
//
// The quote was checked against the note's raw text, but what's on screen is rendered markdown or a
// PDF's text layer: markdown marks are gone and spacing differs. So both sides are compared
// without whitespace, case or markdown marks, which also bridges text split across elements.

const SKIP = /[\s*_`#>|~]/
const HIGHLIGHT = 'cited'

/** The characters that count when comparing, lower-cased. */
function squash(text: string): string {
  let out = ''
  for (const ch of text) if (!SKIP.test(ch)) out += ch.toLowerCase()
  return out.replace(/^["'“”‘’]+|["'“”‘’]+$/g, '')
}

/** A Range over `quote` in the text under `root`, or null. Falls back to the quote's opening words
 * when the whole of it can't be found (a line break or a rendered link in the way). */
export function findQuote(root: Node, quote: string): Range | null {
  const nodes: Text[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text)

  // The squashed text, and for each of its characters the node and offset it came from.
  let flat = ''
  const at: [number, number][] = []
  nodes.forEach((node, n) => {
    const data = node.data
    for (let i = 0; i < data.length; i++) {
      if (SKIP.test(data[i])) continue
      flat += data[i].toLowerCase()
      at.push([n, i])
    }
  })

  const words = quote.split(/\s+/).filter(Boolean)
  for (const needle of [quote, words.slice(0, 8).join(' '), words.slice(0, 4).join(' ')]) {
    const wanted = squash(needle)
    if (wanted.length < 8) continue
    const start = flat.indexOf(wanted)
    if (start < 0) continue
    const [fromNode, fromOffset] = at[start]
    const [toNode, toOffset] = at[start + wanted.length - 1]
    const range = document.createRange()
    range.setStart(nodes[fromNode], fromOffset)
    range.setEnd(nodes[toNode], toOffset + 1)
    return range
  }
  return null
}

/** The page (from 1) of a PDF's extracted text that `quote` is on, from its `<!-- page N -->` marks. */
export function pageOf(extracted: string, quote: string): number | null {
  const wanted = squash(quote.split(/\s+/).slice(0, 8).join(' '))
  if (wanted.length < 8) return null
  let page: number | null = null
  for (const part of extracted.split(/(<!-- page \d+ -->)/)) {
    const mark = part.match(/^<!-- page (\d+) -->$/)
    if (mark) page = Number(mark[1])
    else if (squash(part).includes(wanted)) return page ?? 1
  }
  return null
}

/** Marks the range (or clears the mark with null). Uses the CSS highlight API where the web view
 * has it, else selects the text. */
export function mark(range: Range | null) {
  const registry = (globalThis.CSS as { highlights?: Map<string, unknown> } | undefined)?.highlights
  const Highlight = (globalThis as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight
  if (registry && Highlight) {
    if (range) registry.set(HIGHLIGHT, new Highlight(range))
    else registry.delete(HIGHLIGHT)
    return
  }
  const selection = getSelection()
  selection?.removeAllRanges()
  if (range) selection?.addRange(range)
}
