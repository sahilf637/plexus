<script lang="ts">
  import DOMPurify from 'dompurify'
  import { marked } from 'marked'
  import { tick } from 'svelte'
  import { fileUrl, getItem, linkItems, openItemFile, openSource, openUrl, removeAiLink, revertTitle, search, unlinkItems } from '../lib/api'
  import { formatBytes, isAiLink, kindIcon, kindLabel, linkLabel, linkStrength, tierLabel, tiers } from '../lib/kinds'
  import { findQuote, mark, pageOf } from '../lib/quote'
  import { toast } from '../lib/toasts.svelte'
  import type { Concept, Edge, Item, ItemDetail, Topic } from '../lib/types'
  import Icon from './Icon.svelte'
  import Lightbox from './Lightbox.svelte'
  import PdfView from './PdfView.svelte'

  let {
    id,
    items,
    edges,
    onselect,
    onclose,
    onask,
    ondelete,
    concepts,
    activeConcept,
    onconcept,
    path,
    ontopic,
    width,
    minWidth,
    maxWidth,
    wide,
    onwide,
    onresize,
    onresized,
    onreset,
    quote = null,
  }: {
    id: string
    items: Map<string, Item>
    edges: Edge[]
    onselect: (id: string) => void
    onclose: () => void
    /** Opens Ask, limited to this. */
    onask: () => void
    ondelete: () => void
    concepts: Concept[]
    activeConcept: number | null
    onconcept: (concept: Concept) => void
    /** The topics the item sits in, from the top level down. */
    path: Topic[]
    ontopic: (id: string) => void
    /** How wide the panel is, in pixels. */
    width: number
    /** The narrowest and widest it can be dragged. */
    minWidth: number
    maxWidth: number
    /** Wide enough to read the item (the reading layout). */
    wide: boolean
    onwide: () => void
    /** The edge is being dragged to this width. */
    onresize: (width: number) => void
    /** The drag ended. */
    onresized: () => void
    /** Back to the default width. */
    onreset: () => void
    /** A passage to show and mark (a source of an Ask answer). */
    quote?: string | null
  } = $props()

  let dragging = $state(false)
  let drag: { x: number; width: number } | null = null
  const fit = (w: number) => Math.round(Math.min(maxWidth, Math.max(minWidth, w)))

  function gripDown(event: PointerEvent) {
    if (event.button !== 0) return
    drag = { x: event.clientX, width }
    dragging = true
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    event.preventDefault()
  }

  function gripMove(event: PointerEvent) {
    // The panel is on the right, so dragging left makes it wider.
    if (drag) onresize(fit(drag.width + drag.x - event.clientX))
  }

  function gripUp() {
    if (!drag) return
    drag = null
    dragging = false
    onresized()
  }

  function gripKey(event: KeyboardEvent) {
    const by = event.key === 'ArrowLeft' ? 24 : event.key === 'ArrowRight' ? -24 : 0
    if (!by) return
    event.preventDefault()
    onresize(fit(width + by))
    onresized()
  }

  let detail = $state<ItemDetail | null>(null)
  let imageUrl = $state<string | null>(null)
  /** The original, when it's a PDF to show page by page. */
  let pdfUrl = $state<string | null>(null)
  /** The picture shown full size. */
  let zoomed = $state<{ src: string; alt: string } | null>(null)
  let menuOpen = $state(false)
  /** Preview URLs of the extra originals that are images, by position. */
  let extraUrls = $state<(string | null)[]>([])
  let error = $state('')
  let scroller = $state<HTMLDivElement>()
  let heading = $state<HTMLElement>()
  /** The note's own title has scrolled out of view, so the bar on top shows it instead. */
  let titleHidden = $state(false)
  let scrolled = $state(false)

  // Reloads when the item changes in the background too (a link's page was read).
  const version = $derived(`${items.get(id)?.bodyHash}|${items.get(id)?.title}`)

  $effect(() => {
    const current = id
    void version
    detail = null
    imageUrl = null
    pdfUrl = null
    extraUrls = []
    error = ''
    getItem(current)
      .then(async (d) => {
        if (current !== id) return
        detail = d
        if (d.kind === 'image' && d.assetFile) imageUrl = await fileUrl(d.assetFile)
        if (isPdf(d) && d.assetFile) pdfUrl = await fileUrl(d.assetFile)
        extraUrls = await Promise.all(d.attachments.map((a) => (a.kind === 'image' ? fileUrl(a.file) : null)))
      })
      .catch((e) => (error = String(e)))
  })

  /** One link per other item: the one made by hand if there is one, else the closest. */
  const mine = $derived.by(() => {
    const best = new Map<string, Edge>()
    for (const e of edges.filter((e) => e.src === id || e.dst === id)) {
      const other = e.src === id ? e.dst : e.src
      const current = best.get(other)
      if (!current || (e.kind === 'user' && current.kind !== 'user') || (current.kind !== 'user' && e.weight > current.weight)) best.set(other, e)
    }
    return [...best.values()]
  })
  const related = $derived(
    mine
      .map((e) => ({ edge: e, item: items.get(e.src === id ? e.dst : e.src) }))
      .filter((r): r is { edge: Edge; item: Item } => r.item !== undefined)
      .sort((a, b) => b.edge.weight - a.edge.weight),
  )
  /** Related items grouped by how sure the link is, strongest first. */
  const groups = $derived(
    tiers.map((tier) => ({ tier, rows: related.filter((r) => r.edge.tier === tier) })).filter((g) => g.rows.length),
  )

  // Making a link by hand: search, pick.
  let linking = $state(false)
  let linkQuery = $state('')
  let candidates = $state<Item[]>([])
  let linkTimer: ReturnType<typeof setTimeout> | undefined

  $effect(() => {
    const query = linkQuery.trim()
    clearTimeout(linkTimer)
    if (!linking || !query) {
      candidates = []
      return
    }
    linkTimer = setTimeout(async () => {
      const ids = await search(query)
      if (linkQuery.trim() !== query) return
      const linked = new Set(mine.filter((e) => e.kind === 'user').map((e) => (e.src === id ? e.dst : e.src)))
      candidates = ids.filter((i) => i !== id && !linked.has(i)).flatMap((i) => items.get(i) ?? []).slice(0, 6)
    }, 150)
  })

  // A different item starts with the box closed, scrolled to the top.
  $effect(() => {
    void id
    menuOpen = false
    linking = false
    linkQuery = ''
    titleHidden = false
    scrolled = false
    scroller?.scrollTo({ top: 0 })
  })

  // The bar on top names the note once its heading has gone out of sight.
  $effect(() => {
    if (!scroller || !heading) return
    const watch = new IntersectionObserver(([entry]) => (titleHidden = !entry.isIntersecting), { root: scroller })
    watch.observe(heading)
    return () => watch.disconnect()
  })

  async function makeLink(other: string) {
    linking = false
    linkQuery = ''
    await linkItems(id, other)
  }

  const html = $derived(
    detail && (detail.kind === 'text' || detail.kind === 'code' || (detail.kind === 'url' && detail.extracted === 'web@1'))
      ? render(detail.body)
      : '',
  )
  /** The user's note about an image or file: its body without the embedded image. */
  const noteHtml = $derived.by(() => {
    if (!detail || (detail.kind !== 'image' && detail.kind !== 'file')) return ''
    const note = detail.body.replace(/!\[[^\]]*\]\([^)]*\)/g, '').trim()
    return note ? render(note) : ''
  })

  function render(markdown: string): string {
    return DOMPurify.sanitize(marked.parse(markdown, { async: false }))
  }

  const isPdf = (d: ItemDetail) => d.mime === 'application/pdf' || /\.pdf$/i.test(d.assetPath ?? '')

  /** Clicks inside a note's text: web links open in the browser (never in this window), pictures
   * open full size. */
  function contentClick(event: MouseEvent) {
    const target = event.target as HTMLElement
    const link = target.closest('a')
    if (link) {
      event.preventDefault()
      const href = link.getAttribute('href') ?? ''
      if (/^https?:\/\//.test(href)) void openUrl(href)
      return
    }
    if (target instanceof HTMLImageElement) zoomed = { src: target.currentSrc || target.src, alt: target.alt }
  }

  // --- the quoted passage of an Ask source -------------------------------------------------

  /** The PDF page the quote is on, to scroll to. */
  let quotePage = $state<number | null>(null)

  $effect(() => {
    const wanted = quote
    const shown = detail
    mark(null)
    quotePage = null
    if (!wanted || !shown || shown.id !== id) return
    if (pdfUrl && shown.extractedText) {
      quotePage = pageOf(shown.extractedText, wanted)
      return // Marked once that page is drawn (`pdfDrawn`).
    }
    void tick().then(() => {
      const root = scroller?.querySelector('.content')
      // Text found in a file is folded away; open it to show the quote.
      root?.querySelectorAll('details').forEach((d) => (d.open = true))
      const range = root ? findQuote(root, wanted) : null
      if (!range) return
      mark(range)
      range.startContainer.parentElement?.scrollIntoView({ block: 'center', behavior: 'smooth' })
    })
    return () => mark(null)
  })

  function pdfDrawn(page: number, element: HTMLElement) {
    if (!quote || page !== quotePage) return
    const range = findQuote(element, quote)
    if (!range) return
    mark(range)
    range.startContainer.parentElement?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }

  async function copyText() {
    menuOpen = false
    if (!detail) return
    const text = [detail.body, detail.extractedText ?? ''].filter((t) => t.trim()).join('\n\n').replace(/<!-- page \d+ -->\n?/g, '')
    try {
      await navigator.clipboard.writeText(text.trim())
      toast('Copied')
    } catch (e) {
      toast(`Couldn't copy: ${e}`)
    }
  }

  /** The colour of a link's type tag, so it can be read at a glance. */
  const linkTone: Record<string, string> = {
    supports: 'var(--kind-code)',
    contradicts: 'var(--danger)',
    extends: 'var(--accent)',
    example_of: 'var(--kind-url)',
  }

  /** "example.com" from a URL, for the source line. */
  function hostOf(url: string): string {
    try {
      return new URL(url).hostname.replace(/^www\./, '')
    } catch {
      return url
    }
  }

  function capturedLabel(iso: string): string {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }
</script>

<svelte:window
  onpointerdown={(e) => menuOpen && !(e.target as Element).closest('.more') && (menuOpen = false)}
  onkeydowncapture={(e) => {
    // Esc closes the menu first, before the map gets it (which would close the panel).
    if (menuOpen && e.key === 'Escape') {
      e.stopImmediatePropagation()
      menuOpen = false
    }
  }}
/>

{#if zoomed}<Lightbox src={zoomed.src} alt={zoomed.alt} onclose={() => (zoomed = null)} />{/if}

<aside class:wide class:dragging style:width={`${width}px`}>
  <!-- The left edge: drag to set the width, double click for the default, arrows from the keyboard. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="grip"
    role="separator"
    aria-orientation="vertical"
    aria-label="Panel width"
    aria-valuenow={width}
    aria-valuemin={minWidth}
    aria-valuemax={maxWidth}
    tabindex="0"
    title="Drag to resize, double click for the default"
    onpointerdown={gripDown}
    onpointermove={gripMove}
    onpointerup={gripUp}
    onpointercancel={gripUp}
    ondblclick={onreset}
    onkeydown={gripKey}
  ></div>
  <header class:scrolled>
    <span class="chip" style:--kind={detail ? `var(--kind-${detail.kind})` : 'var(--accent)'}>
      {#if detail}<Icon name={kindIcon[detail.kind]} size={13} />{/if}
      {detail ? (isPdf(detail) ? 'PDF' : kindLabel[detail.kind]) : ''}{detail?.language ? ` · ${detail.language}` : ''}
    </span>
    {#if detail && titleHidden}
      <button class="bar-title" onclick={() => scroller?.scrollTo({ top: 0, behavior: 'smooth' })} title="Back to the top">{detail.title}</button>
    {:else}
      <span class="bar-gap"></span>
    {/if}
    <span class="tools">
      <button class="icon-btn" onclick={onask} title="Ask about this" aria-label="Ask about this"><Icon name="sparkles" /></button>
      <button class="icon-btn" onclick={onwide} title={wide ? 'Narrow the panel (E)' : 'Widen to read (E)'} aria-label={wide ? 'Narrow the panel' : 'Widen to read'} aria-pressed={wide}>
        <Icon name={wide ? 'collapse' : 'expand'} />
      </button>
      {#if detail}
        <span class="more">
          <button class="icon-btn" class:on={menuOpen} onclick={() => (menuOpen = !menuOpen)} title="More" aria-label="More" aria-haspopup="menu" aria-expanded={menuOpen}>
            <Icon name="more" />
          </button>
          {#if menuOpen}
            <div class="menu" role="menu">
              {#if detail.sourceUrl}
                <button role="menuitem" onclick={() => { menuOpen = false; openSource(detail!.id) }}><Icon name="external" size={14} />Open the page</button>
              {/if}
              <button role="menuitem" onclick={() => { menuOpen = false; openItemFile(detail!.id) }}>
                <Icon name="file" size={14} />{detail.assetPath ? 'Open the original' : 'Open the note file'}
              </button>
              <button role="menuitem" onclick={copyText}><Icon name="copy" size={14} />Copy text</button>
              <hr />
              <button role="menuitem" class="danger" onclick={() => { menuOpen = false; ondelete() }}><Icon name="trash" size={14} />Delete <kbd>Del</kbd></button>
            </div>
          {/if}
        </span>
      {/if}
      <button class="icon-btn" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
    </span>
  </header>

  <div class="scroll" bind:this={scroller} onscroll={() => (scrolled = (scroller?.scrollTop ?? 0) > 2)}>
  {#if error}
    <p class="error">{error}</p>
  {:else if detail}
    {#if path.length}
      <nav class="path" aria-label="Topics">
        {#each path as step, i (step.id)}
          {#if i > 0}<Icon name="chevron" size={12} />{/if}<button onclick={() => ontopic(step.id)}>{step.label}</button>
        {/each}
      </nav>
    {/if}
    <h2 bind:this={heading}>{detail.title}</h2>
    <p class="meta">
      {capturedLabel(detail.capturedAt)}{#if detail.size} · {formatBytes(detail.size)}{/if}
      {#if detail.titleByAi}
        · <span class="ai" title={detail.titleWas ? `Was: ${detail.titleWas}` : ''}>AI title</span>
        <button class="revert" onclick={() => revertTitle(detail!.id)}>Revert</button>
      {/if}
    </p>
    {#if items.get(id)?.summary}<p class="summary">{items.get(id)?.summary}</p>{/if}
    {#if concepts.length}
      <ul class="concepts" aria-label="Concepts">
        {#each concepts as concept (concept.id)}
          <li>
            <button class:on={activeConcept === concept.id} onclick={() => onconcept(concept)} title="Show everything about this on the map">
              {concept.name}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if detail.kind === 'url' && detail.sourceUrl}
      <button class="source" onclick={() => openSource(detail!.id)} title={detail.sourceUrl}>
        <Icon name="globe" size={13} /><span>{hostOf(detail.sourceUrl)}</span><Icon name="external" size={12} />
      </button>
    {/if}

    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="content" onclick={contentClick}>
      {#if pdfUrl}
        {#if noteHtml}<div class="markdown note">{@html noteHtml}</div>{/if}
        <PdfView src={pdfUrl} page={quotePage} ondrawn={pdfDrawn} />
        {#if detail.extractedText}
          <details class="extracted">
            <summary>Text found</summary>
            <pre>{detail.extractedText.replace(/<!-- page \d+ -->\n?/g, '')}</pre>
          </details>
        {/if}
      {:else if detail.kind === 'image' || detail.kind === 'file'}
        {#if imageUrl}
          <button class="picture" onclick={() => (zoomed = { src: imageUrl!, alt: detail!.title })} title="Show full size">
            <img src={imageUrl} alt={detail.title} />
          </button>
        {:else if detail.kind === 'file'}
          <p class="muted">{detail.mime ?? 'File'}</p>
        {/if}
        {#each detail.attachments as attachment, i}
          {#if extraUrls[i]}
            <button class="picture" onclick={() => (zoomed = { src: extraUrls[i]!, alt: attachment.name })} title="Show full size">
              <img src={extraUrls[i]} alt={attachment.name} />
            </button>
          {:else if attachment.kind !== 'image'}
            <button class="attachment" onclick={() => openItemFile(detail!.id, i)}>
              <Icon name="file" size={15} />
              <span class="name">{attachment.name}</span>
              {#if attachment.size}<span class="muted">{formatBytes(attachment.size)}</span>{/if}
            </button>
          {/if}
        {/each}
        <!-- Sanitised with DOMPurify above. -->
        {#if noteHtml}<div class="markdown note">{@html noteHtml}</div>{/if}
        {#if detail.extractedText}
          <details class="extracted" open={detail.kind === 'file'}>
            <summary>Text found</summary>
            <pre>{detail.extractedText}</pre>
          </details>
        {:else if detail.extracted === null && detail.assetPath}
          <p class="muted">Reading…</p>
        {/if}
      {:else if detail.kind === 'url' && !html}
        {#if detail.extracted === null}<p class="muted">Reading the page…</p>
        {:else}<p class="muted">The page couldn't be read. Open it to see it in the browser.</p>{/if}
      {:else}
        <!-- Sanitised with DOMPurify above. -->
        <div class="markdown">{@html html}</div>
      {/if}
    </div>

    {#snippet rows(list: typeof related)}
      <ul>
        {#each list as { edge, item } (item.id)}
          <li>
            <button onclick={() => onselect(item.id)}>
              <span class="dot" style:background={`var(--kind-${item.kind})`}></span>
              <span class="text">
                <span class="title">{item.title}</span>
                {#if edge.reason || linkLabel(edge.kind, edge.src === id)}
                  <span class="reason">
                    {#if linkLabel(edge.kind, edge.src === id)}<span class="type" style:--tone={linkTone[edge.kind] ?? 'var(--accent)'}>{linkLabel(edge.kind, edge.src === id)}</span>{/if}
                    {edge.reason ?? ''}
                  </span>
                {/if}
              </span>
              <span class="strength" title="How closely related">
                <span style:width={`${linkStrength(edge.weight) * 100}%`}></span>
              </span>
            </button>
            {#if edge.kind === 'user'}
              <button class="unlink" title="Remove this link" aria-label="Remove link" onclick={() => unlinkItems(id, item.id)}>✕</button>
            {:else if isAiLink(edge.kind)}
              <button class="unlink" title="Remove this link" aria-label="Remove link" onclick={() => removeAiLink(id, item.id)}>✕</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/snippet}

    <section>
      {#each groups as group (group.tier)}
        {#if group.tier === 'maybe'}
          <!-- Less sure: folded away unless it's all there is. -->
          <details open={groups.length === 1}>
            <summary><h3>{tierLabel.maybe} <span class="count">{group.rows.length}</span></h3></summary>
            {@render rows(group.rows)}
          </details>
        {:else}
          <h3>{tierLabel[group.tier]} <span class="count">{group.rows.length}</span></h3>
          {@render rows(group.rows)}
        {/if}
      {:else}
        <h3>Related</h3>
        <p class="muted">{items.get(id)?.status === 'pending' ? 'Finding…' : 'Nothing yet'}</p>
      {/each}

      {#if linking}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="input link-search"
          type="text"
          placeholder="Find an item to link"
          autofocus
          bind:value={linkQuery}
          onkeydown={(e) => e.key === 'Escape' && (e.stopPropagation(), (linking = false))}
        />
        <ul>
          {#each candidates as candidate (candidate.id)}
            <li>
              <button onclick={() => makeLink(candidate.id)}>
                <span class="dot" style:background={`var(--kind-${candidate.kind})`}></span>
                <span class="text"><span class="title">{candidate.title}</span></span>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <button class="btn add-link" onclick={() => (linking = true)}><Icon name="link" size={14} />Link to…</button>
      {/if}
    </section>
  {/if}
  </div>
</aside>

<style>
  /* Docked to the right edge, as if it slid in from the side. */
  aside {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    max-width: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
    box-shadow: -12px 0 28px -16px rgb(0 0 0 / 0.45);
    animation: slide-in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
    transition: width 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }

  /* Following the hand, not gliding after it. */
  aside.dragging {
    transition: none;
    user-select: none;
  }

  .grip {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    z-index: 5;
    width: 8px;
    cursor: col-resize;
    touch-action: none;
    outline: none;
  }

  .grip::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 2px;
    background: var(--accent);
    opacity: 0;
    transition: opacity 120ms;
  }

  .grip:hover::after,
  .grip:focus-visible::after,
  aside.dragging .grip::after {
    opacity: 0.8;
  }

  @keyframes slide-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    aside {
      animation: none;
      transition: none;
    }
  }

  /* The bar stays put while the note scrolls underneath it. */
  header {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px 8px 16px;
    background: var(--surface);
    transition: box-shadow 150ms;
  }

  header.scrolled {
    box-shadow: 0 8px 10px -8px rgb(0 0 0 / 0.35);
    z-index: 1;
  }

  .bar-gap {
    flex: 1;
  }

  .bar-title {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--text);
    font-weight: 600;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    animation: bar-in 150ms ease-out;
  }

  @keyframes bar-in {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .scroll {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 16px 16px;
    overflow-y: auto;
  }

  .chip {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    padding: 3px 9px 3px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--kind) 14%, transparent);
    color: var(--kind);
  }

  .tools {
    flex: none;
    display: flex;
    gap: 2px;
    margin-right: -6px;
  }

  .path {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .path button {
    padding: 0 2px;
    border: none;
    background: none;
    color: var(--text-muted);
  }

  .path button:hover {
    color: var(--text);
  }

  h2 {
    margin: 4px 0 0;
    font-size: 18px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .ai {
    color: var(--accent);
  }

  .revert {
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
    text-decoration: underline;
  }

  .revert:hover {
    color: var(--text);
  }

  .summary {
    margin: 2px 0 0;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .concepts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
  }

  .concepts li {
    display: block;
  }

  .concepts button {
    padding: 2px 10px;
    border: none;
    border-radius: 999px;
    background: var(--fill);
    font-size: 12px;
  }

  .concepts button:hover {
    background: var(--fill-hover);
  }

  .concepts button.on {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .type {
    margin-right: 4px;
    padding: 0 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--tone) 16%, transparent);
    color: var(--tone);
    font-size: 11px;
  }

  .meta,
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin: 0;
  }

  .content {
    margin-top: 10px;
  }

  .content img {
    display: block;
    max-width: 100%;
    border-radius: 8px;
  }

  .attachment + .attachment {
    margin-top: 8px;
  }

  .attachment {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: none;
    border-radius: 8px;
    background: var(--fill);
    text-align: left;
    color: var(--text-muted);
  }

  .attachment .name {
    color: var(--text);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Where a saved page came from: its site, opening the page. */
  .source {
    align-self: flex-start;
    max-width: 100%;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    padding: 2px 8px 2px 6px;
    border: none;
    border-radius: 999px;
    background: var(--fill);
    color: var(--text-muted);
    font-size: 12px;
  }

  .source span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source:hover {
    background: var(--fill-hover);
    color: var(--text);
  }

  /* The "More" menu under its button. */
  .more {
    position: relative;
  }

  .icon-btn.on {
    background: var(--hover);
    color: var(--text);
  }

  .menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 3;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    border-radius: 10px;
    background: var(--popover);
    box-shadow: var(--shadow);
    animation: pop 120ms ease-out;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  .menu button {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border: none;
    border-radius: 6px;
    background: none;
    text-align: left;
  }

  .menu button:hover {
    background: var(--hover);
  }

  .menu .danger {
    color: var(--danger);
  }

  .menu .danger:hover {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }

  .menu kbd {
    margin-left: auto;
  }

  .menu hr {
    width: 100%;
    margin: 4px 0;
    border: none;
    border-top: 1px solid var(--line);
  }

  .picture {
    display: block;
    width: 100%;
    padding: 0;
    border: none;
    background: none;
    cursor: zoom-in;
  }

  .picture + .picture,
  .picture + .attachment,
  .attachment + .picture {
    margin-top: 8px;
  }

  /* A note's text, set for reading. */
  .markdown {
    overflow-wrap: anywhere;
    line-height: 1.6;
  }

  .markdown :global(:first-child) {
    margin-top: 0;
  }

  .markdown :global(:is(h1, h2, h3, h4, h5, h6)) {
    margin: 1.4em 0 0.4em;
    line-height: 1.3;
    letter-spacing: -0.01em;
  }

  .markdown :global(h1) {
    font-size: 1.35em;
  }

  .markdown :global(h2) {
    font-size: 1.2em;
  }

  .markdown :global(:is(h3, h4, h5, h6)) {
    font-size: 1.05em;
  }

  .markdown :global(:is(p, ul, ol, blockquote, pre, table, figure)) {
    margin: 0 0 0.85em;
  }

  .markdown :global(:is(ul, ol)) {
    padding-left: 1.4em;
  }

  .markdown :global(li + li) {
    margin-top: 0.25em;
  }

  .markdown :global(a) {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--accent) 40%, transparent);
    text-underline-offset: 2px;
    cursor: pointer;
  }

  .markdown :global(a:hover) {
    text-decoration-color: currentColor;
  }

  .markdown :global(img) {
    display: block;
    max-width: 100%;
    height: auto;
    margin: 0.4em auto;
    border-radius: 8px;
    cursor: zoom-in;
  }

  .markdown :global(blockquote) {
    padding: 2px 0 2px 14px;
    border-left: 3px solid var(--accent-soft);
    color: var(--text-muted);
  }

  .markdown :global(hr) {
    margin: 1.4em 0;
    border: none;
    border-top: 1px solid var(--line);
  }

  .markdown :global(code) {
    padding: 1px 5px;
    border-radius: 5px;
    background: var(--fill);
    font: 0.88em var(--mono);
  }

  .markdown :global(pre) {
    overflow-x: auto;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--surface-2);
    font: 12px/1.5 var(--mono);
  }

  .markdown :global(pre code) {
    padding: 0;
    background: none;
    font: inherit;
  }

  .markdown :global(table) {
    display: block;
    max-width: 100%;
    overflow-x: auto;
    border-collapse: collapse;
    font-size: 0.92em;
  }

  .markdown :global(:is(th, td)) {
    padding: 5px 10px;
    border-bottom: 1px solid var(--line);
    text-align: left;
    vertical-align: top;
  }

  .markdown :global(th) {
    font-weight: 600;
  }

  /* Wide: a reading column with a comfortable measure, larger text. */
  aside.wide .scroll {
    padding: 8px 32px 32px;
  }

  aside.wide .scroll > :global(*) {
    width: 100%;
    max-width: 720px;
    margin-left: auto;
    margin-right: auto;
  }

  aside.wide .source {
    width: auto;
    margin-left: max(0px, calc((100% - 720px) / 2));
  }

  aside.wide h2 {
    font-size: 26px;
    letter-spacing: -0.02em;
  }

  aside.wide .summary {
    font-size: 15px;
  }

  aside.wide .markdown {
    font-size: 16px;
    line-height: 1.7;
  }

  aside.wide .content {
    margin-top: 18px;
  }

  aside.wide section {
    margin-top: 28px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
  }

  .note {
    margin-top: 10px;
  }

  .extracted {
    margin-top: 10px;
  }

  .extracted summary {
    font-size: 12px;
  }

  .extracted pre {
    margin: 6px 0 0;
    max-height: 320px;
    overflow: auto;
    padding: 10px;
    border-radius: 8px;
    background: var(--surface-2);
    font: 12px/1.5 var(--mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  section {
    margin-top: 14px;
  }

  h3 {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  ul + h3,
  ul + details,
  section > details + h3 {
    margin-top: 12px;
  }

  .count {
    color: var(--text-muted);
    font-weight: 400;
  }

  summary {
    cursor: pointer;
    color: var(--text-muted);
  }

  summary h3 {
    display: inline;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .unlink {
    border: none;
    background: none;
    color: var(--text-muted);
    padding: 4px 6px;
    font-size: 12px;
    opacity: 0;
  }

  li:hover .unlink,
  .unlink:focus-visible {
    opacity: 1;
  }

  .add-link {
    margin-top: 12px;
    height: 28px;
    font-size: 13px;
  }

  .link-search {
    margin-top: 10px;
  }

  li button:first-child {
    flex: 1;
    min-width: 0;
    width: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 6px;
    border: none;
    border-radius: 6px;
    background: none;
    text-align: left;
  }

  li button:hover {
    background: var(--hover);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .title,
  .reason {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reason {
    color: var(--text-muted);
    font-size: 12px;
  }

  .error {
    color: var(--danger);
  }

  .strength {
    width: 32px;
    height: 3px;
    flex-shrink: 0;
    border-radius: 2px;
    background: var(--surface-2);
    overflow: hidden;
  }

  .strength span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
</style>
