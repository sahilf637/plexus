<script lang="ts">
  import { onMount } from 'svelte'
  import {
    captureBytes,
    captureFile,
    captureText,
    deleteItems,
    enrichStatus,
    getMap,
    hotkeyStatus,
    on,
    openItemFile,
    restoreItems,
    saveLayout,
    search,
    showCapture,
  } from './lib/api'
  import { resolve, snapshot } from './lib/paste'
  import { toast } from './lib/toasts.svelte'
  import type { CaptureResult, Concept, EnrichStatus, Item, ItemConcepts, ItemLinked, LinksChanged, MapData, Tier } from './lib/types'
  import Icon from './components/Icon.svelte'
  import EmptyState from './components/EmptyState.svelte'
  import ItemPanel from './components/ItemPanel.svelte'
  import MapView from './components/Map.svelte'
  import type { PanDirection } from './lib/graph/map'
  import SettingsDialog from './components/SettingsDialog.svelte'
  import Toasts from './components/Toasts.svelte'

  let hotkey = $state('Ctrl+Alt+V')
  let settingsOpen = $state(false)
  let settingsSection = $state<'general' | 'ai'>('general')
  let data = $state<MapData | null>(null)
  /** Bumped to rebuild the map from scratch (e.g. after the library changed on disk). */
  let mapVersion = $state(0)
  let mapView = $state<MapView>()
  let selected = $state<string | null>(null)
  let query = $state('')
  let matches = $state<string[] | null>(null)
  let searchInput = $state<HTMLInputElement>()
  /** Items picked together with Shift-click (two or more; a single pick is just `selected`). */
  let marked = $state<string[]>([])
  /** The right-click menu: for this item, at these pixels in the map. */
  let menu = $state<{ id: string; x: number; y: number } | null>(null)
  /** What each item is about, from the AI. */
  let conceptsByItem = $state<Record<string, Concept[]>>({})
  /** The concept whose items are highlighted on the map. */
  let activeConcept = $state<Concept | null>(null)
  let aiStatus = $state<EnrichStatus>({ state: 'off', pending: 0, message: null })
  /** Delete's Undo stays on screen longer than other messages. */
  const UNDO_MS = 10000

  // Which links the map draws, remembered on this computer.
  const SHOW_KEY = 'plexus.showLinks'
  const showOptions: { tier: Tier; label: string; hint: string }[] = [
    { tier: 'strong', label: 'Strong', hint: 'Show only the links Plexus is sure about' },
    { tier: 'related', label: 'Related', hint: 'Show strong and related links' },
    { tier: 'maybe', label: 'All', hint: 'Show every link, including maybes' },
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
  const conceptItems = $derived(
    activeConcept ? Object.keys(conceptsByItem).filter((id) => conceptsByItem[id].some((c) => c.id === activeConcept!.id)) : null,
  )
  /** What the map keeps bright: search results, else the active concept's items. */
  const highlight = $derived(matches ?? conceptItems)

  function toggleConcept(concept: Concept) {
    activeConcept = activeConcept?.id === concept.id ? null : concept
  }

  function onItemConcepts(event: ItemConcepts) {
    conceptsByItem[event.id] = event.concepts
    if (activeConcept && !conceptItems?.length) activeConcept = null
  }

  async function refreshHotkey() {
    const status = await hotkeyStatus()
    if (status.trigger) hotkey = status.trigger
  }

  async function loadMap() {
    data = await getMap()
    const byItem: Record<string, Concept[]> = {}
    for (const concept of data.concepts ?? [])
      for (const id of concept.items) (byItem[id] ??= []).push({ id: concept.id, name: concept.name })
    conceptsByItem = byItem
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

  /** A link was made or removed by hand: redraw the links of the items it touches. */
  function onLinksChanged(event: LinksChanged) {
    if (!data) return
    const touched = new Set(event.ids)
    data.edges = [...data.edges.filter((e) => !touched.has(e.src) && !touched.has(e.dst)), ...event.edges]
    mapView?.setEdges(data.edges)
  }

  /** An item's note changed in the background (a link's page was read): new title, body. */
  function onItemUpdated(item: Item) {
    if (!data) return
    const index = data.items.findIndex((i) => i.id === item.id)
    if (index < 0) return
    data.items[index] = item
    mapView?.addItem(item)
  }

  /** A click on an item or on empty space; with Shift an item joins or leaves the selection. */
  function pick(id: string | null, additive = false) {
    menu = null
    if (id === null) {
      selected = null
      marked = []
    } else if (!additive) {
      selected = id
      marked = []
    } else {
      const current = marked.length ? marked : selected ? [selected] : []
      const next = current.includes(id) ? current.filter((i) => i !== id) : [...current, id]
      marked = next.length > 1 ? next : []
      selected = next.length === 1 ? next[0] : null
    }
  }

  /** Room the menu needs, so it stays inside the map near the window's edges. */
  const MENU_SIZE = { width: 190, height: 90 }
  let mainEl = $state<HTMLElement>()

  function openMenu(id: string, x: number, y: number) {
    if (!marked.includes(id)) pick(id)
    const room = { width: mainEl?.clientWidth ?? Infinity, height: mainEl?.clientHeight ?? Infinity }
    menu = {
      id,
      x: Math.max(0, Math.min(x, room.width - MENU_SIZE.width)),
      y: Math.max(0, Math.min(y, room.height - MENU_SIZE.height)),
    }
  }

  /** Deletes items (their files go to the library's trash) and offers Undo. */
  async function remove(ids: string[]) {
    menu = null
    if (!ids.length) return
    const first = itemsById.get(ids[0])?.title
    try {
      const result = await deleteItems(ids)
      if (!result.ids.length) return
      const message = result.ids.length === 1 ? `Deleted “${first}”` : `Deleted ${result.ids.length} items`
      toast(message, { label: 'Undo', run: () => undo(result.batch) }, UNDO_MS)
    } catch (e) {
      toast(`Couldn't delete: ${e}`)
    }
  }

  async function undo(batch: string) {
    try {
      const count = await restoreItems(batch)
      toast(count === 1 ? 'Restored' : `Restored ${count} items`)
    } catch (e) {
      toast(`Couldn't restore: ${e}`)
    }
  }

  /** Items were deleted (here or by anything else): they leave the map and the selection. */
  function onItemsDeleted(ids: string[]) {
    if (!data) return
    const gone = new Set(ids)
    mapView?.removeItems(ids)
    data.items = data.items.filter((i) => !gone.has(i.id))
    data.edges = data.edges.filter((e) => !gone.has(e.src) && !gone.has(e.dst))
    if (selected && gone.has(selected)) selected = null
    const left = marked.filter((id) => !gone.has(id))
    marked = left.length > 1 ? left : []
    if (left.length === 1) selected = left[0]
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
        if (pasted.kind === 'text')
          report(await captureText(pasted.text, false, pasted.html), () => captureText(pasted.text, true, pasted.html))
        else if (pasted.kind === 'blob') report(await captureBytes(pasted.blob, pasted.name))
        else report(await captureFile(pasted.path))
      }
    } catch (e) {
      toast(`Couldn't save: ${e}`)
    }
  }

  function onkeydown(event: KeyboardEvent) {
    const typing = isTyping(event.target, 'input, textarea')
    if (event.key === 'Escape' && !typing) {
      activeConcept = null
      selected = null
      marked = []
      menu = null
    }
    if (event.key === 'Delete' && !typing && !settingsOpen) {
      remove(marked.length ? marked : selected ? [selected] : [])
    }
    if (event.key === '/' && !typing) {
      event.preventDefault()
      searchInput?.focus()
    }
    navigate(event)
  }

  const PAN_KEYS: Record<string, PanDirection> = {
    ArrowLeft: 'left',
    ArrowRight: 'right',
    ArrowUp: 'up',
    ArrowDown: 'down',
  }

  /** Keys that move around the map: arrows pan (Shift: faster), + and - zoom, 0 shows
   * everything, F centres the selected item. They stay out of the way of text fields, the
   * settings dialog and the side panel (which scrolls with the arrows). */
  function mapKeys(event: KeyboardEvent): boolean {
    if (!data || settingsOpen || event.ctrlKey || event.metaKey || event.altKey) return false
    return !isTyping(event.target, 'input, textarea, select, dialog, aside, [role="listbox"]')
  }

  function navigate(event: KeyboardEvent) {
    if (!mapKeys(event)) return
    const direction = PAN_KEYS[event.key]
    if (direction) {
      event.preventDefault()
      mapView?.panKey(direction, true, event.shiftKey)
      return
    }
    switch (event.key) {
      case '+':
      case '=':
        mapView?.zoom(true)
        break
      case '-':
      case '_':
        mapView?.zoom(false)
        break
      case '0':
        mapView?.showAll()
        break
      case 'f':
      case 'F':
        mapView?.showSelected()
        break
    }
  }

  function onkeyup(event: KeyboardEvent) {
    const direction = PAN_KEYS[event.key]
    if (direction) mapView?.panKey(direction, false)
  }

  onMount(() => {
    refreshHotkey()
    loadMap()
    enrichStatus().then((status) => (aiStatus = status))
    // The portal may still be waiting for the user to approve the shortcut at startup.
    const timer = setTimeout(refreshHotkey, 3000)
    const unsubscribers = [
      on<Item>('item-added', onItemAdded),
      on<ItemLinked>('item-linked', onItemLinked),
      on<LinksChanged>('links-changed', onLinksChanged),
      on<Item>('item-updated', onItemUpdated),
      on<ItemConcepts>('item-concepts', onItemConcepts),
      on<EnrichStatus>('enrich-status', (status) => (aiStatus = status)),
      on<string[]>('items-deleted', onItemsDeleted),
      on('library-changed', loadMap),
    ]
    return () => {
      clearTimeout(timer)
      unsubscribers.forEach((u) => u.then((off) => off()))
    }
  })
</script>

<svelte:window
  {onpaste}
  {onkeydown}
  {onkeyup}
  onblur={() => mapView?.stopPan()}
  onpointerdown={(event) => {
    if (menu && !isTyping(event.target, '.menu')) menu = null
  }}
/>

<div class="app">
  <header>
    <span class="brand"><span class="mark" aria-hidden="true"></span>Plexus</span>
    {#if data?.items.length}
      <div class="search">
        <Icon name="search" size={15} />
        <input
          bind:this={searchInput}
          bind:value={query}
          onkeydown={onSearchKey}
          type="text"
          placeholder="Search"
          aria-label="Search"
        />
        {#if matches}<span class="count">{matches.length}</span>{:else}<kbd>/</kbd>{/if}
      </div>
      {#if activeConcept}
        <button class="concept-pill" onclick={() => (activeConcept = null)} title="Stop highlighting (Esc)">
          {activeConcept.name}
          <Icon name="close" size={12} />
        </button>
      {/if}
    {/if}
    <div class="actions">
      {#if data?.edges.length}
        <div class="segmented" role="radiogroup" aria-label="Links shown">
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
        <button
          class="icon-btn"
          onclick={tidy}
          disabled={tidying}
          title="Tidy layout: group related items together"
          aria-label="Tidy layout"
        >
          <Icon name="layout" />
        </button>
      {/if}
      {#if aiStatus.state === 'working' || (aiStatus.state === 'idle' && aiStatus.pending > 0)}
        <span class="ai-pill" title="AI is adding titles, concepts and links"><span class="spin"></span>{aiStatus.pending}</span>
      {:else if aiStatus.state === 'paused'}
        <button class="ai-pill warn" title={aiStatus.message ?? ''} onclick={() => { settingsSection = 'ai'; settingsOpen = true }}>AI paused</button>
      {/if}
      <button class="btn primary" onclick={showCapture}><Icon name="plus" size={15} />Add <kbd>{hotkey}</kbd></button>
      <button class="icon-btn" onclick={() => { settingsSection = 'general'; settingsOpen = true }} title="Settings" aria-label="Settings">
        <Icon name="settings" />
      </button>
    </div>
  </header>

  <main bind:this={mainEl}>
    {#if data && data.items.length}
      {#key mapVersion}
        <MapView
          bind:this={mapView}
          {data}
          {selected}
          matches={highlight}
          {lowestTier}
          {marked}
          onselect={pick}
          onmenu={openMenu}
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
          ondelete={() => remove([selected!])}
          concepts={conceptsByItem[selected] ?? []}
          activeConcept={activeConcept?.id ?? null}
          onconcept={toggleConcept}
        />
      {/if}
      {#if marked.length > 1}
        <div class="bar" role="status">
          <span class="n">{marked.length} selected</span>
          <button class="btn danger" onclick={() => remove(marked)}><Icon name="trash" size={14} />Delete <kbd>Del</kbd></button>
          <button class="icon-btn" onclick={() => (marked = [])} title="Clear selection (Esc)" aria-label="Clear selection">
            <Icon name="close" size={14} />
          </button>
        </div>
      {/if}
      {#if menu}
        {@const target = itemsById.get(menu.id)}
        {@const count = marked.length > 1 ? marked.length : 1}
        <div class="menu" role="menu" style:left={`${menu.x}px`} style:top={`${menu.y}px`}>
          {#if count === 1 && target}
            <button role="menuitem" onclick={() => { openItemFile(target.id); menu = null }}>
              <span><Icon name="file" size={14} />{target.assetPath ? 'Open file' : 'Open note'}</span>
            </button>
          {/if}
          <button role="menuitem" class="danger" onclick={() => remove(count > 1 ? marked : [menu!.id])}>
            <span><Icon name="trash" size={14} />{count > 1 ? `Delete ${count} items` : 'Delete'}</span> <kbd>Del</kbd>
          </button>
        </div>
      {/if}
    {:else if data}
      <EmptyState {hotkey} onadd={showCapture} />
    {/if}
    <Toasts />
  </main>
</div>

{#if settingsOpen}
  <SettingsDialog
    section={settingsSection}
    onclose={() => {
      settingsOpen = false
      refreshHotkey()
      enrichStatus().then((status) => (aiStatus = status))
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
    height: 52px;
    padding: 0 14px 0 16px;
    background: var(--bg);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .mark {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: radial-gradient(circle at 30% 30%, color-mix(in srgb, var(--accent) 60%, white), var(--accent));
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .search {
    flex: 1;
    max-width: 380px;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--text-muted);
    transition: box-shadow 100ms;
  }

  .search:focus-within {
    box-shadow: 0 0 0 2px var(--accent);
    color: var(--text);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: none;
    color: var(--text);
    font: inherit;
    outline: none;
  }

  .concept-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px 0 10px;
    border: none;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 13px;
    font-weight: 500;
  }

  /* Status, not a button: a dot and a few words. */
  .ai-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .ai-pill.warn {
    color: var(--warning);
    font-weight: 600;
  }

  .spin {
    width: 10px;
    height: 10px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation-duration: 3s;
    }
  }

  .count {
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
  }

  .actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .actions .segmented {
    margin-right: 6px;
  }

  .menu,
  .bar {
    position: absolute;
    z-index: 5;
    display: flex;
    border-radius: 10px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .menu {
    flex-direction: column;
    min-width: 170px;
    padding: 4px;
  }

  .menu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 6px 10px;
    border: none;
    border-radius: 6px;
    background: none;
    text-align: left;
  }

  .menu button span {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .menu button:hover {
    background: var(--hover);
  }

  .menu .danger:hover {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }

  .bar {
    left: 50%;
    bottom: 18px;
    transform: translateX(-50%);
    align-items: center;
    gap: 8px;
    padding: 6px;
    padding-left: 14px;
  }

  .bar .n {
    margin-right: 6px;
    font-weight: 500;
  }

  .danger {
    color: var(--danger);
  }

  main {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
</style>
