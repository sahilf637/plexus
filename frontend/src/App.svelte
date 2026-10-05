<script lang="ts">
  import { onMount } from 'svelte'
  import {
    captureBytes,
    captureFile,
    captureText,
    getMap,
    hotkeyStatus,
    on,
    saveLayout,
    search,
    showCapture,
  } from './lib/api'
  import { resolve, snapshot } from './lib/paste'
  import { toast } from './lib/toasts.svelte'
  import type { CaptureResult, Item, ItemLinked, MapData, Tier } from './lib/types'
  import EmptyState from './components/EmptyState.svelte'
  import ItemPanel from './components/ItemPanel.svelte'
  import MapView from './components/Map.svelte'
  import SettingsDialog from './components/SettingsDialog.svelte'
  import Toasts from './components/Toasts.svelte'

  let hotkey = $state('Ctrl+Alt+V')
  let settingsOpen = $state(false)
  let data = $state<MapData | null>(null)
  /** Bumped to rebuild the map from scratch (e.g. after the library changed on disk). */
  let mapVersion = $state(0)
  let mapView = $state<MapView>()
  let selected = $state<string | null>(null)
  let query = $state('')
  let matches = $state<string[] | null>(null)
  let searchInput = $state<HTMLInputElement>()

  // Which links the map draws, remembered on this computer.
  const SHOW_KEY = 'plexus.showLinks'
  const showOptions: { tier: Tier; label: string; hint: string }[] = [
    { tier: 'strong', label: 'Strong', hint: 'Only links Plexus is sure about' },
    { tier: 'related', label: 'Related', hint: 'Strong and related links' },
    { tier: 'maybe', label: 'All', hint: 'Every link, including maybes' },
  ]
  let lowestTier = $state<Tier>(readShow())
  function readShow(): Tier {
    try {
      const saved = localStorage.getItem(SHOW_KEY)
      if (saved === 'strong' || saved === 'related' || saved === 'maybe') return saved
    } catch {}
    return 'related'
  }
  function setShow(tier: Tier) {
    lowestTier = tier
    try {
      localStorage.setItem(SHOW_KEY, tier)
    } catch {}
  }

  const itemsById = $derived(new Map((data?.items ?? []).map((item) => [item.id, item])))

  async function refreshHotkey() {
    const status = await hotkeyStatus()
    if (status.trigger) hotkey = status.trigger
  }

  async function loadMap() {
    data = await getMap()
    mapVersion++
  }

  function onItemAdded(item: Item) {
    if (!data || itemsById.has(item.id)) return
    const wasEmpty = data.items.length === 0
    data.items.push(item)
    // An empty library shows the welcome screen instead of the map; the map then mounts with
    // this first item. Otherwise the live map adds it (with a pulse).
    if (!wasEmpty) mapView?.addItem(item)
  }

  function onItemLinked(event: ItemLinked) {
    if (!data) return
    const item = data.items.find((i) => i.id === event.id)
    if (!item) return
    const hadLinks = data.edges.some((e) => e.src === event.id || e.dst === event.id)
    item.status = event.status
    data.edges = [...data.edges.filter((e) => e.src !== event.id && e.dst !== event.id), ...event.edges]
    mapView?.setEdges(data.edges)
    // A newly linked item moves next to what it's related to; settled items stay put.
    if (!hadLinks && event.edges.length) mapView?.settle(event.id)
  }

  /** An item's note changed in the background (a link's page was read): new title, body. */
  function onItemUpdated(item: Item) {
    if (!data) return
    const index = data.items.findIndex((i) => i.id === item.id)
    if (index < 0) return
    data.items[index] = item
    mapView?.addItem(item)
  }

  let tidying = $state(false)
  async function tidy() {
    tidying = true
    await mapView?.tidy()
    tidying = false
  }

  // Search as you type; everything that doesn't match is dimmed on the map.
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  $effect(() => {
    const current = query.trim()
    clearTimeout(searchTimer)
    if (!current) {
      matches = null
      return
    }
    searchTimer = setTimeout(async () => {
      const ids = await search(current)
      if (query.trim() === current) matches = ids
    }, 150)
  })

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === 'Enter' && matches?.length) selected = matches[0]
    if (event.key === 'Escape') {
      query = ''
      searchInput?.blur()
    }
  }

  function report(result: CaptureResult, retryAsSecret?: () => Promise<CaptureResult>) {
    if (result.status === 'saved') toast(`Saved “${result.title}”`)
    else if (result.status === 'duplicate')
      toast(`Already saved as “${result.title}”`, { label: 'Show', run: () => (selected = result.id) })
    else if (retryAsSecret)
      toast(`Not saved: this looks like ${result.reason}.`, {
        label: 'Save anyway',
        run: async () => report(await retryAsSecret()),
      })
  }

  /** Whether the event happened inside one of these elements (e.g. the search box). */
  function isTyping(target: EventTarget | null, selector: string): boolean {
    return target instanceof Element && target.closest(selector) !== null
  }

  // Ctrl+V on the map saves straight away, without the capture window.
  async function onpaste(event: ClipboardEvent) {
    if (!event.clipboardData || isTyping(event.target, 'input, textarea, dialog')) return
    event.preventDefault()
    try {
      for (const pasted of await resolve(snapshot(event.clipboardData))) {
        if (pasted.kind === 'text') report(await captureText(pasted.text), () => captureText(pasted.text, true))
        else if (pasted.kind === 'blob') report(await captureBytes(pasted.blob, pasted.name))
        else report(await captureFile(pasted.path))
      }
    } catch (e) {
      toast(`Couldn't save: ${e}`)
    }
  }

  function onkeydown(event: KeyboardEvent) {
    const typing = isTyping(event.target, 'input, textarea')
    if (event.key === 'Escape' && !typing) selected = null
    if (event.key === '/' && !typing) {
      event.preventDefault()
      searchInput?.focus()
    }
  }

  onMount(() => {
    refreshHotkey()
    loadMap()
    // The portal may still be waiting for the user to approve the shortcut at startup.
    const timer = setTimeout(refreshHotkey, 3000)
    const unsubscribers = [
      on<Item>('item-added', onItemAdded),
      on<ItemLinked>('item-linked', onItemLinked),
      on<Item>('item-updated', onItemUpdated),
      on('library-changed', loadMap),
    ]
    return () => {
      clearTimeout(timer)
      unsubscribers.forEach((u) => u.then((off) => off()))
    }
  })
</script>

<svelte:window {onpaste} {onkeydown} />

<div class="app">
  <header>
    <span class="brand">Plexus</span>
    {#if data?.items.length}
      <div class="search">
        <input
          bind:this={searchInput}
          bind:value={query}
          onkeydown={onSearchKey}
          type="search"
          placeholder="Search  /"
          aria-label="Search"
        />
        {#if matches}<span class="count">{matches.length} found</span>{/if}
      </div>
    {/if}
    <div class="actions">
      {#if data?.edges.length}
        <div class="show" role="radiogroup" aria-label="Links shown">
          <span class="show-label">Links</span>
          {#each showOptions as option}
            <button
              role="radio"
              aria-checked={lowestTier === option.tier}
              class:on={lowestTier === option.tier}
              title={option.hint}
              onclick={() => setShow(option.tier)}>{option.label}</button
            >
          {/each}
        </div>
        <button onclick={tidy} disabled={tidying} title="Lay the map out again so related items group together">
          {tidying ? 'Tidying…' : 'Tidy layout'}
        </button>
      {/if}
      <button onclick={showCapture}>Add <kbd>{hotkey}</kbd></button>
      <button onclick={() => (settingsOpen = true)}>Settings</button>
    </div>
  </header>

  <main>
    {#if data && data.items.length}
      {#key mapVersion}
        <MapView
          bind:this={mapView}
          {data}
          {selected}
          {matches}
          {lowestTier}
          onselect={(id) => (selected = id)}
          onplaced={(positions) => saveLayout(positions)}
        />
      {/key}
      {#if selected && itemsById.has(selected)}
        <ItemPanel
          id={selected}
          items={itemsById}
          edges={data.edges}
          onselect={(id) => (selected = id)}
          onclose={() => (selected = null)}
        />
      {/if}
    {:else if data}
      <EmptyState {hotkey} onadd={showCapture} />
    {/if}
    <Toasts />
  </main>
</div>

{#if settingsOpen}
  <SettingsDialog
    onclose={() => {
      settingsOpen = false
      refreshHotkey()
    }}
  />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .brand {
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .search {
    flex: 1;
    max-width: 420px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .search input {
    flex: 1;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    outline: none;
  }

  .search input:focus {
    border-color: var(--accent);
  }

  .count {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .actions {
    margin-left: auto;
    display: flex;
    gap: 8px;
  }

  .actions button {
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  .actions button:hover {
    border-color: var(--accent);
  }

  .show {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  .show-label {
    padding: 0 6px 0 8px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .actions .show button {
    padding: 3px 9px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text-muted);
    font-size: 13px;
  }

  .actions .show button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }

  main {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
</style>
