<script lang="ts">
  // A PDF's pages, drawn with pdf.js as they scroll into view, each with an invisible text layer
  // on top so text can be selected and a quoted passage found. pdf.js is loaded only when the
  // first PDF is opened.
  import { untrack } from 'svelte'
  import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from 'pdfjs-dist'

  let {
    src,
    page = null,
    onready,
    ondrawn,
  }: {
    /** A URL the web view can fetch the file from. */
    src: string
    /** Scrolls to this page (from 1) once it's known. */
    page?: number | null
    /** The pages are laid out (their text layers fill in as they're drawn). */
    onready?: (pages: number) => void
    /** A page was drawn, with its text in `element`. */
    ondrawn?: (page: number, element: HTMLElement) => void
  } = $props()

  /** How long the width must hold still before pages are drawn again (the panel animates it). */
  const SETTLE_MS = 150

  let host = $state<HTMLDivElement>()
  let width = $state(0)
  /** `width` once it stopped changing: the width pages are drawn at. */
  let settled = $state(0)
  /** Each page's size at scale 1, in PDF points; the first page's until a page is drawn. */
  let sizes = $state<{ w: number; h: number }[]>([])
  let error = $state('')
  let doc: PDFDocumentProxy | null = null
  let task: PDFDocumentLoadingTask | null = null
  /** Pages in or near view, and the width each was last drawn at. */
  const visible = new Set<number>()
  const drawnAt = new Map<number, number>()
  /** Pages waiting to be drawn, one at a time, and the drawing in progress. */
  const waiting: number[] = []
  let busy = false
  let rendering: { n: number; task: RenderTask } | null = null
  /** The page boxes, bound as they're made. */
  let slots = $state<HTMLDivElement[]>([])

  async function pdfjs() {
    const lib = await import('pdfjs-dist/legacy/build/pdf.mjs')
    const worker = await import('pdfjs-dist/legacy/build/pdf.worker.min.mjs?url')
    lib.GlobalWorkerOptions.workerSrc = worker.default
    return lib
  }

  $effect(() => {
    const url = src
    let cancelled = false
    sizes = []
    error = ''
    visible.clear()
    drawnAt.clear()
    waiting.length = 0
    ;(async () => {
      try {
        const lib = await pdfjs()
        const loading = lib.getDocument({
          url,
          standardFontDataUrl: '/pdfjs/standard_fonts/',
          cMapUrl: '/pdfjs/cmaps/',
          wasmUrl: '/pdfjs/wasm/',
        })
        task = loading
        const loaded = await loading.promise
        if (cancelled) return
        doc = loaded
        // Every page is taken to be the first one's size, so the scroll height is right before
        // anything is drawn; a page that differs is measured when it is drawn. (Reading all of
        // them up front takes long in a big PDF.)
        const first = (await loaded.getPage(1)).getViewport({ scale: 1 })
        sizes = Array.from({ length: loaded.numPages }, () => ({ w: first.width, h: first.height }))
        onready?.(loaded.numPages)
      } catch (e) {
        if (!cancelled) error = String(e).replace(/^Error:\s*/, '')
      }
    })()
    return () => {
      cancelled = true
      rendering?.task.cancel()
      void task?.destroy()
      task = null
      doc = null
    }
  })

  // Draw pages as they come near the view.
  $effect(() => {
    const boxes = slots.filter(Boolean)
    if (!sizes.length || !host || boxes.length < sizes.length) return
    const watch = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const n = Number((entry.target as HTMLElement).dataset.page)
          if (entry.isIntersecting) {
            visible.add(n)
            want(n)
          } else {
            // Far out of view: let the drawing go (a long PDF would otherwise keep every page's
            // canvas); it's drawn again when scrolled back to.
            visible.delete(n)
            drawnAt.delete(n)
            if (rendering?.n === n) rendering.task.cancel()
            ;(entry.target as HTMLElement).replaceChildren()
          }
        }
      },
      { rootMargin: '800px 0px' },
    )
    boxes.forEach((slot) => watch.observe(slot))
    return () => watch.disconnect()
  })

  // The width counts once it holds still: the panel animates it, and drawing at every step
  // would draw each page a dozen times. The pages stretch to the new width meanwhile.
  $effect(() => {
    const now = width
    if (!untrack(() => settled)) {
      settled = now
      return
    }
    const timer = setTimeout(() => (settled = now), SETTLE_MS)
    return () => clearTimeout(timer)
  })

  // A new width redraws what's showing; the rest redraw when scrolled to.
  $effect(() => {
    void settled
    untrack(() => {
      rendering?.task.cancel()
      for (const n of visible) want(n)
    })
  })

  // Jumps, not glides: a glide to a far page would draw every page on the way.
  $effect(() => {
    if (page && sizes.length) untrack(() => slots[Math.min(page, sizes.length) - 1])?.scrollIntoView({ block: 'start' })
  })

  function want(n: number) {
    if (!waiting.includes(n)) waiting.push(n)
    void pump()
  }

  /** Draws the waiting pages one after another, skipping any that scrolled away meanwhile. */
  async function pump() {
    if (busy) return
    busy = true
    try {
      while (waiting.length) {
        waiting.sort((a, b) => a - b)
        const n = waiting.shift()!
        if (visible.has(n)) await draw(n).catch((e) => console.warn('PDF page failed', n, e))
      }
    } finally {
      busy = false
    }
  }

  async function draw(n: number) {
    const slot = untrack(() => slots[n - 1])
    const at = settled
    if (!doc || !slot || !at || drawnAt.get(n) === at) return
    const pdfPage = await doc.getPage(n)
    const base = pdfPage.getViewport({ scale: 1 })
    if (base.width !== sizes[n - 1].w || base.height !== sizes[n - 1].h) sizes[n - 1] = { w: base.width, h: base.height }
    const scale = at / base.width
    const viewport = pdfPage.getViewport({ scale })
    // Sharp on a dense screen, but not more pixels than the eye can use.
    const ratio = Math.min(window.devicePixelRatio || 1, 2)

    const canvas = document.createElement('canvas')
    canvas.width = Math.floor(viewport.width * ratio)
    canvas.height = Math.floor(viewport.height * ratio)
    const render = pdfPage.render({
      canvas,
      viewport,
      transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined,
    })
    rendering = { n, task: render }
    try {
      await render.promise
    } catch {
      return // Cancelled: the page left the view or the width changed.
    } finally {
      if (rendering?.task === render) rendering = null
    }
    if (!visible.has(n) || settled !== at) return

    const { TextLayer } = await pdfjs()
    const text = document.createElement('div')
    text.className = 'textLayer'
    slot.style.setProperty('--scale-factor', String(scale))
    slot.style.setProperty('--total-scale-factor', String(scale))
    await new TextLayer({ textContentSource: pdfPage.streamTextContent(), container: text, viewport }).render()
    if (!visible.has(n) || settled !== at) return
    slot.replaceChildren(canvas, text)
    drawnAt.set(n, at)
    pdfPage.cleanup()
    ondrawn?.(n, text)
  }
</script>

<div class="pdf" bind:this={host} bind:clientWidth={width}>
  {#if error}
    <p class="error">Couldn't show this PDF: {error}</p>
  {:else if !sizes.length}
    <div class="page loading" style:aspect-ratio="0.77"></div>
  {/if}
  {#each sizes as size, i}
    <div class="page" data-page={i + 1} bind:this={slots[i]} style:aspect-ratio={`${size.w} / ${size.h}`}></div>
  {/each}
</div>

<style>
  .pdf {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .page {
    position: relative;
    width: 100%;
    overflow: hidden;
    border-radius: 4px;
    background: white;
    box-shadow: 0 0 0 1px var(--line), var(--shadow-sm);
  }

  .page.loading {
    opacity: 0.5;
  }

  .page :global(canvas) {
    display: block;
    width: 100%;
    height: 100%;
  }

  .error {
    color: var(--danger);
    font-size: 13px;
  }

  /* pdf.js's text layer (from its pdf_viewer.css): invisible text placed over the drawn page. */
  .page :global(.textLayer) {
    position: absolute;
    inset: 0;
    overflow: clip;
    line-height: 1;
    text-align: initial;
    letter-spacing: normal;
    word-spacing: normal;
    text-size-adjust: none;
    forced-color-adjust: none;
    transform-origin: 0 0;
    --min-font-size: 1;
    --text-scale-factor: calc(var(--total-scale-factor) * var(--min-font-size));
    --min-font-size-inv: calc(1 / var(--min-font-size));
  }

  .page :global(.textLayer span),
  .page :global(.textLayer br) {
    position: absolute;
    color: transparent;
    white-space: pre;
    cursor: text;
    transform-origin: 0% 0%;
    user-select: text;
  }

  .page :global(.textLayer > :not(.markedContent)),
  .page :global(.textLayer .markedContent span:not(.markedContent)) {
    z-index: 1;
    --font-height: 0;
    font-size: calc(var(--text-scale-factor) * var(--font-height));
    --scale-x: 1;
    --rotate: 0deg;
    transform: rotate(var(--rotate)) scaleX(var(--scale-x)) scale(var(--min-font-size-inv));
  }

  .page :global(.textLayer .markedContent) {
    display: contents;
  }

  .page :global(.textLayer ::selection) {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: transparent;
  }
</style>
