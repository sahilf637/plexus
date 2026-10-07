<script lang="ts">
  import DOMPurify from 'dompurify'
  import { marked } from 'marked'
  import { tick } from 'svelte'
  import { fileUrl, getItem, linkItems, openItemFile, openSource, openUrl, removeAiLink, search, setTitle, unlinkItems } from '../lib/api'
  import { formatBytes, isAiLink, kindIcon, kindLabel, linkLabel, linkStrength } from '../lib/kinds'
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
  /** The strongest link of each tier first, then by closeness. */
  const ranked = $derived(
    [...related].sort((a, b) => ['strong', 'related', 'maybe'].indexOf(a.edge.tier) - ['strong', 'related', 'maybe'].indexOf(b.edge.tier) || b.edge.weight - a.edge.weight),
  )

  /** What the panel shows under the title: the item itself, what's known about it, or what it connects to. */
  type Tab = 'content' | 'about' | 'related'
  let tab = $state<Tab>('content')

  // Renaming: the title turns into a box. Whatever is saved is the user's from then on.
  let editingTitle = $state(false)
  let draft = $state('')
  let titleInput = $state<HTMLTextAreaElement>()

  async function startEdit() {
    if (!detail) return
    draft = detail.title
    editingTitle = true
    await tick()
    fitTitle()
    titleInput?.select()
  }

  /** Grows the box to its text, so a long title wraps the way it did before editing. */
  function fitTitle() {
    if (!titleInput) return
    titleInput.style.height = 'auto'
    titleInput.style.height = `${titleInput.scrollHeight + 2}px`
  }

  function finishEdit(save: boolean) {
    if (!editingTitle) return
    editingTitle = false
    const title = draft.trim()
    if (save && detail && title && title !== detail.title) setTitle(id, title).catch((e) => toast(`Couldn't rename: ${e}`))
  }

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
    tab = 'content'
    editingTitle = false
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
    // The text read from a picture lives in About; everything else is quoted in the content.
    tab = shown.kind === 'image' && !pdfUrl && shown.extractedText ? 'about' : 'content'
    if (pdfUrl && shown.extractedText) {
      quotePage = pageOf(shown.extractedText, wanted)
      return // Marked once that page is drawn (`pdfDrawn`).
    }
    void tick().then(() => {
      const root = scroller?.querySelector(tab === 'about' ? '.about' : '.content')
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

  /** A concept's tint: the same name always gets the same one, picked from a few calm hues. */
  const CONCEPT_HUES = [215, 265, 330, 15, 40, 95, 150, 180]
  function conceptHue(name: string): number {
    let h = 0
    for (const ch of name.toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) >>> 0
    return CONCEPT_HUES[h % CONCEPT_HUES.length]
  }

  /** "example.com" from a URL, for the source line. */
  function hostOf(url: string): string {
    try {
      return new URL(url).hostname.replace(/^www\./, '')
    } catch {
      return url
    }
  }

  function shortDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }

  function capturedLabel(iso: string): string {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (menuOpen && !(e.target as Element).closest('.more')) menuOpen = false
  }}
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
    <div class="head">
      <div class="title-row">
        {#if editingTitle}
          <textarea
            class="title-input"
            rows="1"
            bind:this={titleInput}
            bind:value={draft}
            maxlength="80"
            aria-label="Title"
            oninput={fitTitle}
            onblur={() => finishEdit(true)}
            onkeydown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                finishEdit(true)
              }
              if (e.key === 'Escape') {
                e.stopPropagation()
                finishEdit(false)
              }
            }}
          ></textarea>
        {:else}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <h2 bind:this={heading} ondblclick={startEdit} title="Double-click to edit">{detail.title}</h2>
        {/if}
        <button class="icon-btn edit" class:editing={editingTitle} onclick={startEdit} title="Edit title" aria-label="Edit title" tabindex={editingTitle ? -1 : 0}><Icon name="edit" size={14} /></button>
      </div>
      <p class="meta">
        {#if editingTitle}Enter to save · Esc to cancel{:else}{shortDate(detail.capturedAt)}{#if path.length}{' · '}{path[path.length - 1].label}{/if}{/if}
      </p>
    </div>

    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={tab === 'content'} class:on={tab === 'content'} onclick={() => (tab = 'content')}>Content</button>
      <button role="tab" aria-selected={tab === 'about'} class:on={tab === 'about'} onclick={() => (tab = 'about')}>About</button>
      <button role="tab" aria-selected={tab === 'related'} class:on={tab === 'related'} onclick={() => (tab = 'related')}>
        Related{#if ranked.length}<span class="count">{ranked.length}</span>{/if}
      </button>
    </div>

    {#if detail.kind === 'url' && detail.sourceUrl}
      <button class="source" class:off={tab !== 'content'} onclick={() => openSource(detail!.id)} title={detail.sourceUrl}>
        <Icon name="globe" size={13} /><span>{hostOf(detail.sourceUrl)}</span><Icon name="external" size={12} />
      </button>
    {/if}

    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="content" class:off={tab !== 'content'} onclick={contentClick}>
      {#if pdfUrl}
        {#if noteHtml}<div class="markdown note">{@html noteHtml}</div>{/if}
        <PdfView src={pdfUrl} page={quotePage} ondrawn={pdfDrawn} />
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
        <!-- A file with no picture to show is its text. For a picture the text read from it is in About. -->
        {#if detail.extractedText && detail.kind === 'file'}
          <details class="extracted" open>
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

    {#if tab === 'about'}
      <div class="about">
        {#if items.get(id)?.summary}
          <section>
            <h3><Icon name="sparkles" size={12} />Summary</h3>
            <p class="summary">{items.get(id)?.summary}</p>
          </section>
        {/if}
        {#if concepts.length}
          <section>
            <h3>Concepts</h3>
            <ul class="concepts" aria-label="Concepts">
              {#each concepts as concept (concept.id)}
                <li>
                  <button class:on={activeConcept === concept.id} style:--hue={conceptHue(concept.name)} onclick={() => onconcept(concept)} title="Show everything about this on the map">{concept.name}</button>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
        <dl class="facts">
          <dt>Added</dt>
          <dd>{capturedLabel(detail.capturedAt)}</dd>
          {#if detail.size}<dt>Size</dt><dd>{formatBytes(detail.size)}</dd>{/if}
          {#if path.length}
            <dt>Topic</dt>
            <dd class="trail">
              {#each path as step, i (step.id)}{#if i > 0}<Icon name="chevron" size={11} />{/if}<button onclick={() => ontopic(step.id)}>{step.label}</button>{/each}
            </dd>
          {/if}
        </dl>
        {#if detail.extractedText && (pdfUrl || detail.kind === 'image')}
          <details class="extracted">
            <summary>{pdfUrl ? 'Text read from the document' : 'Text read from the picture'}</summary>
            <pre>{detail.extractedText.replace(/<!-- page \d+ -->\n?/g, '')}</pre>
          </details>
        {/if}
      </div>
    {:else if tab === 'related'}
      <div class="related">
        {#if ranked.length}
          <ul>
            {#each ranked as { edge, item } (item.id)}
              {@const label = linkLabel(edge.kind, edge.src === id)}
              <li>
                <button onclick={() => onselect(item.id)}>
                  <span class="dot" style:background={`var(--kind-${item.kind})`}></span>
                  <span class="text">
                    <span class="title">{item.title}</span>
                    {#if edge.reason || label}
                      <span class="reason">
                        {#if label}<span class="type" style:--tone={linkTone[edge.kind] ?? 'var(--accent)'}>{label}</span>{/if}
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
        {:else}
          <p class="muted">{items.get(id)?.status === 'pending' ? 'Finding…' : 'Nothing yet'}</p>
        {/if}

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
      </div>
    {/if}
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

  .head {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 4px;
  }

  .head .meta {
    margin: 0;
  }

  /* Content, About, Related: one thing at a time, each with the whole width. */
  .tabs {
    display: flex;
    gap: 18px;
    margin: 6px 0 8px;
    border-bottom: 1px solid var(--line);
  }

  .tabs button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: auto;
    padding: 6px 0 7px;
    margin-bottom: -1px;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    color: var(--text-muted);
    font-size: 13px;
  }

  .tabs button:hover {
    color: var(--text);
  }

  .tabs button.on {
    border-bottom-color: var(--accent);
    color: var(--text);
  }

  .off {
    display: none !important;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .title-row {
    display: flex;
    align-items: flex-start;
    gap: 4px;
  }

  .title-row h2 {
    flex: 0 1 auto;
    min-width: 0;
  }

  /* Out of the way until the title is hovered or focused. */
  .edit {
    flex: none;
    margin-top: -3px;
    opacity: 0;
    transition: opacity 120ms;
  }

  .title-row:hover .edit,
  .edit:focus-visible {
    opacity: 1;
  }

  /* The title made editable where it stands: the padding and border are pulled back out with
   * negative margins, so the text, and everything under it, stays put. */
  .title-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    margin: -4px -8px;
    padding: 3px 7px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--fill);
    color: var(--text);
    font: inherit;
    font-size: 16px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.3;
    resize: none;
    overflow: hidden;
    outline: none;
    overflow-wrap: anywhere;
    transition: border-color 100ms;
  }

  .title-input:hover,
  .title-input:focus {
    border-color: color-mix(in srgb, var(--text-muted) 45%, transparent);
  }

  .edit.editing {
    visibility: hidden;
  }

  .summary {
    margin: 4px 0 0;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .about,
  .related {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding-top: 6px;
  }

  .about h3 {
    display: flex;
    align-items: center;
    gap: 5px;
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

  /* Each concept gets its own soft tint, mixed with the text colour so it reads in light and dark. */
  .concepts li button:first-child {
    --tone: hsl(var(--hue) 65% 55%);
    padding: 2px 10px;
    border: none;
    border-radius: 999px;
    background: color-mix(in srgb, var(--tone) 16%, transparent);
    color: color-mix(in srgb, var(--tone) 65%, var(--text));
    font-size: 12px;
  }

  .concepts li button:first-child:hover {
    background: color-mix(in srgb, var(--tone) 26%, transparent);
  }

  /* The one filtering the map: a stronger fill and a thin outline of its own colour. */
  .concepts li button:first-child.on {
    background: color-mix(in srgb, var(--tone) 30%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--tone) 70%, transparent);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 16px;
    margin: 0;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    font-size: 13px;
  }

  .facts dt {
    color: var(--text-muted);
  }

  .facts dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .facts .trail {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
  }

  .facts .trail button {
    padding: 0 2px;
    border: none;
    background: none;
    color: var(--accent);
    height: auto;
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

  aside.wide .title-input {
    font-size: 26px;
    letter-spacing: -0.02em;
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

  h3 {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .count {
    color: var(--text-muted);
    font-weight: 400;
  }

  summary {
    cursor: pointer;
    color: var(--text-muted);
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
    margin-top: 4px;
    height: 28px;
    font-size: 13px;
    align-self: flex-start;
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

  /* The Related tab has the room to say why. */
  .related .reason {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    white-space: normal;
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
