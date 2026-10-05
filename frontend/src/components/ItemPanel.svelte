<script lang="ts">
  import DOMPurify from 'dompurify'
  import { marked } from 'marked'
  import { fileUrl, getItem, linkItems, openItemFile, openSource, search, unlinkItems } from '../lib/api'
  import { formatBytes, kindLabel, linkStrength, tierLabel, tiers } from '../lib/kinds'
  import type { Edge, Item, ItemDetail } from '../lib/types'

  let {
    id,
    items,
    edges,
    onselect,
    onclose,
    ondelete,
  }: {
    id: string
    items: Map<string, Item>
    edges: Edge[]
    onselect: (id: string) => void
    onclose: () => void
    ondelete: () => void
  } = $props()

  let detail = $state<ItemDetail | null>(null)
  let imageUrl = $state<string | null>(null)
  /** Preview URLs of the extra originals that are images, by position. */
  let extraUrls = $state<(string | null)[]>([])
  let error = $state('')

  // Reloads when the item changes in the background too (a link's page was read).
  const version = $derived(items.get(id)?.bodyHash)

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

  // A different item starts with the box closed.
  $effect(() => {
    void id
    linking = false
    linkQuery = ''
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

  function capturedLabel(iso: string): string {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }
</script>

<aside>
  <header>
    <span class="chip">{detail ? kindLabel[detail.kind] : ''}{detail?.language ? ` · ${detail.language}` : ''}</span>
    <button class="close" onclick={onclose} aria-label="Close">✕</button>
  </header>

  {#if error}
    <p class="error">{error}</p>
  {:else if detail}
    <h2>{detail.title}</h2>
    <p class="meta">
      Saved {capturedLabel(detail.capturedAt)}
      {#if detail.size}· {formatBytes(detail.size)}{/if}
    </p>

    <div class="actions">
      {#if detail.sourceUrl}<button onclick={() => openSource(detail!.id)}>Open link</button>{/if}
      <button onclick={() => openItemFile(detail!.id)}>{detail.assetPath ? 'Open file' : 'Open note'}</button>
      <button class="delete" onclick={ondelete} title="Move to the trash. You can undo right after.">
        Delete <kbd>Del</kbd>
      </button>
    </div>

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
              <span class="name">{attachment.name}</span>
              {#if attachment.size}<span class="muted">{formatBytes(attachment.size)}</span>{/if}
              <span class="open">Open</span>
            </button>
          {/if}
        {/each}
        <!-- Sanitised with DOMPurify above. -->
        {#if noteHtml}<div class="markdown note">{@html noteHtml}</div>{/if}
        {#if detail.extractedText}
          <details class="extracted" open={detail.kind === 'file'}>
            <summary>Text found in the {detail.kind === 'image' ? 'image' : 'file'}</summary>
            <pre>{detail.extractedText}</pre>
          </details>
        {:else if detail.extracted === null && detail.assetPath}
          <p class="muted">Reading the {detail.kind === 'image' ? 'image' : 'file'}…</p>
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
                {#if edge.reason}<span class="reason">{edge.reason}</span>{/if}
              </span>
              <span class="strength" title="How closely related">
                <span style:width={`${linkStrength(edge.weight) * 100}%`}></span>
              </span>
            </button>
            {#if edge.kind === 'user'}
              <button class="unlink" title="Remove this link" aria-label="Remove link" onclick={() => unlinkItems(id, item.id)}>✕</button>
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
        <p class="muted">{items.get(id)?.status === 'pending' ? 'Finding related items…' : 'Nothing related yet.'}</p>
      {/each}

      {#if linking}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="link-search"
          type="search"
          placeholder="Search for an item to link"
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
        <button class="add-link" onclick={() => (linking = true)}>Link to…</button>
      {/if}
    </section>
  {/if}
</aside>

<style>
  aside {
    position: absolute;
    top: 12px;
    right: 12px;
    bottom: 12px;
    width: min(380px, calc(100% - 24px));
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .chip {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .close {
    border: none;
    background: none;
    color: var(--text-muted);
    padding: 4px 6px;
  }

  h2 {
    margin: 0;
    font-size: 17px;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .meta,
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin: 0;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .actions button {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    font-size: 13px;
  }

  .actions .delete {
    margin-left: auto;
    color: var(--danger);
  }

  .content {
    border-top: 1px solid var(--border);
    padding-top: 10px;
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
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    text-align: left;
  }

  .attachment .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .attachment .open {
    color: var(--accent);
    font-size: 12px;
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
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }

  h3 {
    margin: 0 0 6px;
    font-size: 13px;
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
  }

  .unlink {
    border: none;
    background: none;
    color: var(--text-muted);
    padding: 4px 6px;
    font-size: 12px;
  }

  .add-link {
    margin-top: 10px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    font-size: 13px;
  }

  .link-search {
    width: 100%;
    margin-top: 10px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    outline: none;
  }

  .link-search:focus {
    border-color: var(--accent);
  }

  li button:first-child {
    flex: 1;
    width: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px;
    border: none;
    border-radius: 6px;
    background: none;
    text-align: left;
  }

  li button:hover {
    background: var(--surface-2);
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
    width: 48px;
    height: 4px;
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
