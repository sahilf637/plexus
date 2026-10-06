<script lang="ts">
  // A button in the header that opens the topics as a list you can fold open, for jumping to one
  // and for anyone who would rather read a list than a map.
  import type { Topic, TopicsData } from '../lib/types'
  import Icon from './Icon.svelte'

  let { topics, selected, onpick }: { topics: TopicsData; selected: string | null; onpick: (id: string) => void } = $props()

  let open = $state(false)
  let opened = $state<Set<string>>(new Set())
  let root = $state<HTMLElement>()

  const childrenOf = $derived.by(() => {
    const found = new Map<string | null, Topic[]>()
    const ids = new Set(topics.topics.map((t) => t.id))
    for (const t of topics.topics) {
      const key = t.parentId && ids.has(t.parentId) ? t.parentId : null
      found.set(key, [...(found.get(key) ?? []), t])
    }
    for (const list of found.values()) list.sort((a, b) => b.count - a.count || a.label.localeCompare(b.label))
    return found
  })

  function toggle(id: string) {
    const next = new Set(opened)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    opened = next
  }

  function pick(id: string) {
    open = false
    onpick(id)
  }

  function outside(event: PointerEvent) {
    if (open && root && !root.contains(event.target as Node)) open = false
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={(e) => open && e.key === 'Escape' && ((open = false), e.stopPropagation())} />

{#snippet branch(list: Topic[], depth: number)}
  <ul>
    {#each list as topic (topic.id)}
      {@const kids = childrenOf.get(topic.id) ?? []}
      <li>
        <div class="row" class:on={selected === topic.id} style:padding-left={`${6 + depth * 14}px`}>
          {#if kids.length}
            <button class="fold" onclick={() => toggle(topic.id)} aria-label={opened.has(topic.id) ? 'Fold' : 'Unfold'} aria-expanded={opened.has(topic.id)}>
              <Icon name={opened.has(topic.id) ? 'chevron-down' : 'chevron'} size={13} />
            </button>
          {:else}
            <span class="fold"></span>
          {/if}
          <button class="name" onclick={() => pick(topic.id)}>
            <span class="text">{topic.label}</span>
            <span class="n">{topic.count}</span>
          </button>
        </div>
        {#if kids.length && opened.has(topic.id)}
          {@render branch(kids, depth + 1)}
        {/if}
      </li>
    {/each}
  </ul>
{/snippet}

<div class="wrap" bind:this={root}>
  <button class="icon-btn" class:on={open} onclick={() => (open = !open)} title="Topics" aria-label="Topics" aria-expanded={open}>
    <Icon name="tree" />
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label="Topics">
      {@render branch(childrenOf.get(null) ?? [], 0)}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
  }

  .icon-btn.on {
    background: var(--fill);
  }

  .pop {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 20;
    width: 300px;
    max-height: min(480px, calc(100vh - 100px));
    overflow-y: auto;
    padding: 6px;
    border-radius: 12px;
    background: var(--popover);
    box-shadow: var(--shadow), 0 0 0 1px var(--line);
    animation: pop 110ms ease-out;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.98);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pop {
      animation: none;
    }
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .row {
    display: flex;
    align-items: center;
    border-radius: 8px;
  }

  .row:hover,
  .row.on {
    background: var(--fill);
  }

  .fold {
    flex: none;
    width: 22px;
    height: 28px;
    display: grid;
    place-items: center;
    border: none;
    background: none;
    color: var(--text-muted);
  }

  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 28px;
    padding: 0 8px 0 2px;
    border: none;
    background: none;
    text-align: left;
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
</style>
