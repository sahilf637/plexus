<script lang="ts">
  import DOMPurify from 'dompurify'
  import { marked } from 'marked'
  import { fileUrl, getItem, linkItems, openItemFile, openSource, removeAiLink, revertTitle, search, unlinkItems } from '../lib/api'
  import { formatBytes, isAiLink, kindIcon, kindLabel, linkLabel, linkStrength, tierLabel, tiers } from '../lib/kinds'
  import type { Concept, Edge, Item, ItemDetail, Topic } from '../lib/types'
  import Icon from './Icon.svelte'

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
  } = $props()

  let detail = $state<ItemDetail | null>(null)
  let imageUrl = $state<string | null>(null)
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
    extraUrls = []
    error = ''
    getItem(current)
      .then(async (d) => {
        if (current !== id) return
        detail = d
        if (d.kind === 'image' && d.assetFile) imageUrl = await fileUrl(d.assetFile)
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

  /** The colour of a link's type tag, so it can be read at a glance. */
  const linkTone: Record<string, string> = {
    supports: 'var(--kind-code)',
    contradicts: 'var(--danger)',
    extends: 'var(--accent)',
    example_of: 'var(--kind-url)',
  }

  function capturedLabel(iso: string): string {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }
</script>

<aside>
  <header class:scrolled>
    <span class="chip" style:--kind={detail ? `var(--kind-${detail.kind})` : 'var(--accent)'}>
      {#if detail}<Icon name={kindIcon[detail.kind]} size={13} />{/if}
      {detail ? kindLabel[detail.kind] : ''}{detail?.language ? ` · ${detail.language}` : ''}
    </span>
    {#if detail && titleHidden}
      <button class="bar-title" onclick={() => scroller?.scrollTo({ top: 0, behavior: 'smooth' })} title="Back to the top">{detail.title}</button>
    {:else}
      <span class="bar-gap"></span>
    {/if}
    <span class="tools">
      {#if detail?.sourceUrl}
        <button class="icon-btn" onclick={() => openSource(detail!.id)} title="Open link" aria-label="Open link">
          <Icon name="external" />
        </button>
      {/if}
      {#if detail}
        <button
          class="icon-btn"
          onclick={() => openItemFile(detail!.id)}
          title={detail.assetPath ? 'Open file' : 'Open note'}
          aria-label={detail.assetPath ? 'Open file' : 'Open note'}
        >
          <Icon name="file" />
        </button>
        <button class="icon-btn danger" onclick={ondelete} title="Delete (Del). You can undo right after." aria-label="Delete">
          <Icon name="trash" />
        </button>
      {/if}
      <button class="icon-btn" onclick={onask} title="Ask about this" aria-label="Ask about this"><Icon name="sparkles" /></button>
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

    {#if detail.kind === 'url' && html}<p class="source muted">{detail.sourceUrl}</p>{/if}

    <div class="content">
      {#if detail.kind === 'image' || detail.kind === 'file'}
        {#if imageUrl}
          <img src={imageUrl} alt={detail.title} />
        {:else if detail.kind === 'file'}
          <p class="muted">{detail.mime ?? 'File'}</p>
        {/if}
        {#each detail.attachments as attachment, i}
          {#if extraUrls[i]}
            <img src={extraUrls[i]} alt={attachment.name} />
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
        <p class="source">{detail.sourceUrl}</p>
        {#if detail.extracted === null}<p class="muted">Reading the page…</p>{/if}
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
    width: min(400px, 100%);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
    box-shadow: -12px 0 28px -16px rgb(0 0 0 / 0.45);
    animation: slide-in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
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

  .content img + img,
  .content img + .attachment,
  .attachment + img,
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

  .source {
    overflow-wrap: anywhere;
    margin: 0;
  }

  .markdown :global(pre) {
    overflow-x: auto;
    padding: 10px;
    border-radius: 8px;
    background: var(--surface-2);
    font: 12px/1.5 var(--mono);
  }

  .markdown :global(:first-child) {
    margin-top: 0;
  }

  .markdown {
    overflow-wrap: anywhere;
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
