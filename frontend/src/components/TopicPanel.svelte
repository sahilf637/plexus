<script lang="ts">
  // The panel for a picked topic, docked at the side like an item's: its name (click to
  // rename), where it sits in the tree, its sub-topics and its items, the best connected first.
  import { tick } from 'svelte'
  import type { Edge, Item, Topic, TopicsData } from '../lib/types'
  import Icon from './Icon.svelte'

  let {
    topic,
    topics,
    items,
    edges,
    onselect,
    ontopic,
    onclose,
    onask,
    onrename,
    onreset,
  }: {
    topic: Topic
    topics: TopicsData
    items: Map<string, Item>
    edges: Edge[]
    /** An item in the list was clicked. */
    onselect: (id: string) => void
    /** A topic (an ancestor or a sub-topic) was clicked. */
    ontopic: (id: string) => void
    onclose: () => void
    /** Opens Ask, limited to this. */
    onask: () => void
    onrename: (id: string, name: string) => void
    onreset: (id: string) => void
  } = $props()

  /** Items shown before "Show all". */
  const FIRST = 25

  let scroller = $state<HTMLDivElement>()
  let heading = $state<HTMLElement>()
  let titleHidden = $state(false)
  let scrolled = $state(false)
  let editing = $state(false)
  let draft = $state('')
  let input = $state<HTMLInputElement>()
  let showAll = $state(false)

  const byId = $derived(new Map(topics.topics.map((t) => [t.id, t])))
  /** From the top level down to this topic's parent. */
  const path = $derived.by(() => {
    const found: Topic[] = []
    for (let at = byId.get(topic.parentId ?? ''); at; at = byId.get(at.parentId ?? '')) found.unshift(at)
    return found
  })
  const children = $derived(
    topics.topics.filter((t) => t.parentId === topic.id).sort((a, b) => b.count - a.count || a.label.localeCompare(b.label)),
  )
  /** Every item in the topic and its sub-topics, the most linked first. */
  const members = $derived.by(() => {
    const inside = new Set<string>()
    for (const [item, leaf] of Object.entries(topics.itemTopic)) {
      for (let at = byId.get(leaf); at; at = byId.get(at.parentId ?? '')) {
        if (at.id === topic.id) {
          inside.add(item)
          break
        }
      }
    }
    const links = new Map<string, number>()
    for (const e of edges) {
      if (inside.has(e.src) && inside.has(e.dst) && e.tier !== 'maybe') {
        links.set(e.src, (links.get(e.src) ?? 0) + 1)
        links.set(e.dst, (links.get(e.dst) ?? 0) + 1)
      }
    }
    return [...inside]
      .flatMap((id) => items.get(id) ?? [])
      .sort((a, b) => (links.get(b.id) ?? 0) - (links.get(a.id) ?? 0) || a.title.localeCompare(b.title))
  })
  const shown = $derived(showAll ? members : members.slice(0, FIRST))

  // A different topic starts at the top, not being renamed.
  $effect(() => {
    void topic.id
    editing = false
    showAll = false
    titleHidden = false
    scrolled = false
    scroller?.scrollTo({ top: 0 })
  })

  $effect(() => {
    if (!scroller || !heading) return
    const watch = new IntersectionObserver(([entry]) => (titleHidden = !entry.isIntersecting), { root: scroller })
    watch.observe(heading)
    return () => watch.disconnect()
  })

  async function startRename() {
    draft = topic.label
    editing = true
    await tick()
    input?.select()
  }

  function finishRename(save: boolean) {
    if (!editing) return
    editing = false
    const name = draft.trim()
    if (save && name && name !== topic.label) onrename(topic.id, name)
  }
</script>

<aside>
  <header class:scrolled>
    <span class="chip"><Icon name="tree" size={13} />Topic</span>
    {#if titleHidden}
      <button class="bar-title" onclick={() => scroller?.scrollTo({ top: 0, behavior: 'smooth' })} title="Back to the top">{topic.label}</button>
    {:else}
      <span class="bar-gap"></span>
    {/if}
    <span class="tools">
      <button class="icon-btn" onclick={onask} title="Ask about this" aria-label="Ask about this"><Icon name="sparkles" /></button>
      <button class="icon-btn" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
    </span>
  </header>

  <div class="scroll" bind:this={scroller} onscroll={() => (scrolled = (scroller?.scrollTop ?? 0) > 2)}>
    {#if path.length}
      <nav class="path" aria-label="Where this topic sits">
        {#each path as step (step.id)}
          <button onclick={() => ontopic(step.id)}>{step.label}</button><Icon name="chevron" size={12} />
        {/each}
      </nav>
    {/if}

    {#if editing}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input rename"
        bind:this={input}
        bind:value={draft}
        maxlength="60"
        aria-label="Topic name"
        onblur={() => finishRename(true)}
        onkeydown={(e) => {
          if (e.key === 'Enter') finishRename(true)
          if (e.key === 'Escape') {
            e.stopPropagation()
            finishRename(false)
          }
        }}
      />
    {:else}
      <div class="title">
        <h2 bind:this={heading}>{topic.label}</h2>
        <button class="icon-btn" onclick={startRename} title="Rename" aria-label="Rename"><Icon name="edit" size={15} /></button>
      </div>
    {/if}
    <p class="meta">
      {topic.count} {topic.count === 1 ? 'item' : 'items'}
      {#if topic.labelBy === 'ai'}· <span class="ai">AI name</span>{/if}
      {#if topic.labelBy === 'you'}· named by you <button class="revert" onclick={() => onreset(topic.id)}>Reset</button>{/if}
    </p>
    {#if topic.summary}<p class="summary">{topic.summary}</p>{/if}

    {#if children.length}
      <section>
        <h3>Sub-topics <span class="count">{children.length}</span></h3>
        <ul>
          {#each children as child (child.id)}
            <li>
              <button onclick={() => ontopic(child.id)}>
                <span class="text">{child.label}</span>
                <span class="n">{child.count}</span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section>
      <h3>Items <span class="count">{members.length}</span></h3>
      <ul>
        {#each shown as item (item.id)}
          <li>
            <button onclick={() => onselect(item.id)}>
              <span class="dot" style:background={`var(--kind-${item.kind})`}></span>
              <span class="text">{item.title}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if !showAll && members.length > FIRST}
        <button class="btn more" onclick={() => (showAll = true)}>Show all {members.length}</button>
      {/if}
    </section>
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

  .chip {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    padding: 3px 9px 3px 7px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
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

  .tools {
    flex: none;
    display: flex;
    gap: 2px;
    margin-right: -6px;
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

  .title {
    display: flex;
    align-items: flex-start;
    gap: 4px;
  }

  h2 {
    flex: 1;
    margin: 4px 0 0;
    font-size: 18px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .rename {
    width: 100%;
    font-size: 16px;
    font-weight: 600;
  }

  .meta {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
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

  .count {
    font-weight: 400;
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
    min-height: 32px;
    padding: 4px 8px;
    margin: 0 -8px;
    width: calc(100% + 16px);
    border: none;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  li button:hover {
    background: var(--fill);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .n {
    color: var(--text-muted);
    font-size: 12px;
  }

  .more {
    margin-top: 6px;
  }
</style>
