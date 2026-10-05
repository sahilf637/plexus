<script lang="ts">
  import DOMPurify from 'dompurify'
  import { marked } from 'marked'
  import { fileUrl, getItem, openItemFile, openSource } from '../lib/api'
  import { formatBytes, kindLabel } from '../lib/kinds'
  import type { Edge, Item, ItemDetail } from '../lib/types'

  let {
    id,
    items,
    edges,
    onselect,
    onclose,
  }: {
    id: string
    items: Map<string, Item>
    edges: Edge[]
    onselect: (id: string) => void
    onclose: () => void
  } = $props()

  let detail = $state<ItemDetail | null>(null)
  let imageUrl = $state<string | null>(null)
  let error = $state('')

  $effect(() => {
    const current = id
    detail = null
    imageUrl = null
    error = ''
    getItem(current)
      .then(async (d) => {
        if (current !== id) return
        detail = d
        if (d.kind === 'image' && d.assetFile) imageUrl = await fileUrl(d.assetFile)
      })
      .catch((e) => (error = String(e)))
  })

  const related = $derived(
    edges
      .filter((e) => e.src === id || e.dst === id)
      .map((e) => ({ edge: e, item: items.get(e.src === id ? e.dst : e.src) }))
      .filter((r): r is { edge: Edge; item: Item } => r.item !== undefined)
      .sort((a, b) => b.edge.weight - a.edge.weight),
  )

  const html = $derived(
    detail && (detail.kind === 'text' || detail.kind === 'code')
      ? DOMPurify.sanitize(marked.parse(detail.body, { async: false }))
      : '',
  )

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
    </div>

    <div class="content">
      {#if imageUrl}
        <img src={imageUrl} alt={detail.title} />
      {:else if detail.kind === 'url'}
        <p class="source">{detail.sourceUrl}</p>
      {:else if detail.kind === 'file'}
        <p class="muted">{detail.mime ?? 'File'}. Its text will be readable once document support lands.</p>
      {:else}
        <!-- Sanitised with DOMPurify above. -->
        <div class="markdown">{@html html}</div>
      {/if}
    </div>

    <section>
      <h3>Related</h3>
      {#if related.length}
        <ul>
          {#each related as { edge, item } (item.id)}
            <li>
              <button onclick={() => onselect(item.id)}>
                <span class="dot" style:background={`var(--kind-${item.kind})`}></span>
                <span class="title">{item.title}</span>
                <span class="muted">{Math.round(edge.weight * 100)}%</span>
              </button>
            </li>
          {/each}
        </ul>
      {:else if detail.status === 'pending'}
        <p class="muted">Finding related items…</p>
      {:else}
        <p class="muted">Nothing related yet.</p>
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

  .content {
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }

  .content img {
    max-width: 100%;
    border-radius: 8px;
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

  section {
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }

  h3 {
    margin: 0 0 6px;
    font-size: 13px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li button {
    width: 100%;
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

  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .error {
    color: var(--danger);
  }
</style>
