// A handful of items so `npm run dev:ui` shows a populated map.

import type { ItemDetail, Kind } from '../types'

/** A forgetting curve, drawn inline so the article sample has a picture without the network. */
const curve = `data:image/svg+xml;utf8,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 160"><rect width="320" height="160" fill="#f6f6f8"/><path d="M30 20 V135 H300" stroke="#888" fill="none"/><path d="M30 25 C60 90 110 115 300 122" stroke="#5b5bd6" stroke-width="3" fill="none"/><path d="M30 25 C45 60 70 70 90 72 L90 30 C110 70 140 80 170 82 L170 34 C200 60 250 66 300 68" stroke="#c26a16" stroke-width="2" stroke-dasharray="5 4" fill="none"/><text x="160" y="152" font-size="11" text-anchor="middle" fill="#666" font-family="sans-serif">days since learning</text></svg>',
)}`

const article = `The **forgetting curve** describes how newly learned information fades when there is no attempt to keep it. Hermann Ebbinghaus measured it on himself in the 1880s, memorising lists of nonsense syllables and testing how many he could relearn after a delay.

![Retention falls quickly at first; each review flattens the curve](${curve})

## What the curve says

Most of the loss happens early. Within a day a large part of new material is gone, after which the decline slows. How steep the curve is depends on how meaningful the material is, how it was learned, and on sleep and stress.

> Reviewing at growing intervals resets the curve each time, and each new curve is flatter than the last.

## Fighting it

1. **Spaced repetition**: review just before you would forget.
2. **Active recall**: test yourself instead of rereading.
3. **Connect it**: material linked to what you already know fades more slowly.

| Review | Interval |
| --- | --- |
| First | 1 day |
| Second | 3 days |
| Third | 1 week |

Further reading: [spaced repetition](https://en.wikipedia.org/wiki/Spaced_repetition) and the \`SM-2\` scheduling algorithm.`

/** A small PDF made on the spot: one page per entry, each a list of lines. */
function tinyPdf(pages: string[][]): string {
  const objects: string[] = []
  const pageIds = pages.map((_, i) => 4 + i * 2)
  objects[1] = '<< /Type /Catalog /Pages 2 0 R >>'
  objects[2] = `<< /Type /Pages /Kids [${pageIds.map((id) => `${id} 0 R`).join(' ')}] /Count ${pages.length} >>`
  objects[3] = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
  pages.forEach((lines, i) => {
    const text = lines
      .map((line, j) => `BT /F1 ${j === 0 ? 20 : 12} Tf 72 ${j === 0 ? 720 : 690 - j * 20} Td (${line.replace(/[()\\]/g, '\\$&')}) Tj ET`)
      .join('\n')
    objects[pageIds[i]] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${pageIds[i] + 1} 0 R >>`
    objects[pageIds[i] + 1] = `<< /Length ${text.length} >>\nstream\n${text}\nendstream`
  })
  let pdf = '%PDF-1.4\n'
  const offsets: number[] = []
  for (let id = 1; id < objects.length; id++) {
    offsets[id] = pdf.length
    pdf += `${id} 0 obj\n${objects[id]}\nendobj\n`
  }
  const xref = pdf.length
  pdf += `xref\n0 ${objects.length}\n0000000000 65535 f \n`
  for (let id = 1; id < objects.length; id++) pdf += `${String(offsets[id]).padStart(10, '0')} 00000 n \n`
  pdf += `trailer\n<< /Size ${objects.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`
  return `data:application/pdf;base64,${btoa(pdf)}`
}

const reportPages = [
  ['Q3 board report', 'Revenue grew 18% over the quarter, led by the new team plan.', 'Churn fell to 2.1% after the onboarding changes shipped in July.', 'Hiring: two engineers joined; one designer role is still open.'],
  ['Outlook for Q4', 'We expect growth to slow to about 12% as the launch effect fades.', 'The biggest risk is the delayed capture window on Windows.', 'Next steps: finish the graph view and start the import tools.'],
]
const reportText = reportPages.map((lines, i) => `<!-- page ${i + 1} -->\n${lines.join('\n')}`).join('\n\n')

const samples: [Kind, string, string][] = [
  ['text', 'Ownership rules in Rust', 'Each value has a single owner; when the owner goes out of scope the value is dropped.'],
  ['code', 'fn longest<\'a>(x: &\'a str, y: &\'a str) -> &\'a str', '```rust\nfn longest<\'a>(x: &\'a str, y: &\'a str) -> &\'a str {\n    if x.len() > y.len() { x } else { y }\n}\n```'],
  ['url', 'doc.rust-lang.org/book/ch04-01-what-is-ownership', 'https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html'],
  ['text', 'Sourdough starter feeding schedule', 'Feed 1:1:1 flour, water and starter every 12 hours at room temperature.'],
  ['text', 'Why bread needs steam', 'Steam keeps the crust soft early in the bake so the loaf can expand fully.'],
  ['text', 'Meeting notes: Q4 planning', 'Ship the capture window first; graph view next; hire one designer.'],
  ['image', 'Whiteboard sketch', ''],
  ['file', 'Q3 board report', ''],
  ['text', 'Spaced repetition', 'Reviewing at growing intervals beats cramming for long-term memory.'],
  ['url', 'Forgetting curve', 'https://en.wikipedia.org/wiki/Forgetting_curve'],
]

/** What the AI would have said about the samples, so the map shows concepts and summaries. */
const summaries = [
  'Rust gives each value one owner and frees it when the owner goes out of scope.',
  'A function that returns the longer of two string slices, with explicit lifetimes.',
  'The Rust book chapter on how ownership works.',
  'How often to feed a sourdough starter, and in what proportions.',
  'Steam keeps the crust soft early on so the loaf can rise fully.',
  null,
  null,
  null,
  'Reviewing at growing intervals beats cramming for lasting memory.',
  'How memory fades over time without review.',
]

/** Concepts by sample index. */
export const sampleConcepts: Record<string, number[]> = {
  ownership: [0, 1, 2],
  'borrow checker': [0, 1],
  'sourdough starter': [3, 4],
  'bread baking': [3, 4],
  'spaced repetition': [8, 9],
  memory: [8, 9],
}

export function sampleItems(): ItemDetail[] {
  const start = Date.parse('2026-10-01T09:00:00Z')
  return samples.map(([kind, title, body], i) => fill({
    id: `sample-${i}`,
    path: `items/2026/10/sample-${i}.md`,
    kind,
    title,
    sourceUrl: kind === 'url' ? body : null,
    language: kind === 'code' ? 'rust' : null,
    capturedAt: new Date(start + i * 3_600_000).toISOString(),
    bodyHash: `sample-${i}`,
    assetPath: null,
    assetHash: null,
    mime: kind === 'image' ? 'image/png' : kind === 'file' ? 'application/pdf' : null,
    size: null,
    status: 'linked',
    extracted: kind === 'url' ? 'web@1' : null,
    summary: summaries[i] ?? null,
    titleByAi: false,
    titleWas: null,
    body,
    assetFile: null,
    attachments: [],
  }))
}

/** The article and the PDF get their real content. */
function fill(item: ItemDetail): ItemDetail {
  if (item.sourceUrl?.includes('Forgetting_curve')) return { ...item, body: article }
  if (item.kind === 'file')
    return { ...item, assetPath: 'items/2026/10/q3-board-report.pdf', assetFile: tinyPdf(reportPages), extractedText: reportText, extracted: 'pdf-text@1', size: 48_213 }
  return item
}
