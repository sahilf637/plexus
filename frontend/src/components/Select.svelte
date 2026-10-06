<script lang="ts" module>
  export interface Option {
    value: string
    label: string
    /** Small muted text at the end of the row ("sees images"). */
    hint?: string
  }
</script>

<script lang="ts">
  // A dropdown that looks like the rest of the app instead of the system's. The list floats
  // above everything (fixed position, so a scrolling panel doesn't clip it), is searchable when
  // long, and works from the keyboard: arrows, Enter, Escape, or just start typing.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'

  let {
    value,
    options,
    onchange,
    label,
    placeholder = 'Choose…',
    width = 220,
    disabled = false,
  }: {
    value: string
    options: Option[]
    onchange: (value: string) => void
    /** Names the control for screen readers. */
    label: string
    placeholder?: string
    width?: number
    disabled?: boolean
  } = $props()

  /** Lists this long get a search box. */
  const SEARCH_FROM = 10
  const GAP = 6
  const MARGIN = 12
  /** The list grows past the button to fit long names, up to this wide. */
  const MAX_WIDTH = 480

  let open = $state(false)
  let query = $state('')
  let active = $state(0)
  let trigger = $state<HTMLButtonElement>()
  let list = $state<HTMLDivElement>()
  let popover = $state<HTMLDivElement>()
  let place = $state({ left: 0, top: 0, width: 0, maxHeight: 280, up: false })

  const selected = $derived(options.find((o) => o.value === value))
  const searchable = $derived(options.length > SEARCH_FROM)
  const shown = $derived(
    query.trim() ? options.filter((o) => o.label.toLowerCase().includes(query.trim().toLowerCase())) : options,
  )

  async function show() {
    if (disabled || !trigger) return
    const rect = trigger.getBoundingClientRect()
    const below = window.innerHeight - rect.bottom - MARGIN
    const above = rect.top - MARGIN
    const up = below < 180 && above > below
    const popWidth = Math.max(rect.width, 220)
    place = {
      left: Math.max(MARGIN, Math.min(rect.left, window.innerWidth - popWidth - MARGIN)),
      top: up ? rect.top - GAP : rect.bottom + GAP,
      width: popWidth,
      maxHeight: Math.min(300, (up ? above : below) - GAP),
      up,
    }
    query = ''
    active = Math.max(0, options.findIndex((o) => o.value === value))
    open = true
    await tick()
    // It is as wide as its longest name, so it may now run off the screen.
    if (popover) {
      const room = window.innerWidth - popover.offsetWidth - MARGIN
      if (place.left > room) place.left = Math.max(MARGIN, room)
    }
    list?.focus()
    scrollToActive()
  }

  function hide(refocus = true) {
    open = false
    if (refocus) trigger?.focus()
  }

  function choose(option: Option | undefined) {
    if (!option) return
    hide()
    if (option.value !== value) onchange(option.value)
  }

  function scrollToActive() {
    list?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }

  function move(by: number) {
    if (!shown.length) return
    active = (active + by + shown.length) % shown.length
    tick().then(scrollToActive)
  }

  function onlistkey(event: KeyboardEvent) {
    switch (event.key) {
      case 'ArrowDown':
        move(1)
        break
      case 'ArrowUp':
        move(-1)
        break
      case 'Home':
        active = 0
        tick().then(scrollToActive)
        break
      case 'End':
        active = Math.max(0, shown.length - 1)
        tick().then(scrollToActive)
        break
      case 'Enter':
        choose(shown[active])
        break
      case 'Escape':
      case 'Tab':
        hide()
        break
      default:
        return
    }
    // Keys handled here must not reach the dialog (Escape would close it).
    event.preventDefault()
    event.stopPropagation()
  }

  function ontriggerkey(event: KeyboardEvent) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
      event.preventDefault()
      show()
    }
  }

  function outside(event: PointerEvent) {
    if (!open) return
    const target = event.target as Node
    if (!list?.contains(target) && !trigger?.contains(target)) hide(false)
  }
</script>

<svelte:window onpointerdown={outside} onresize={() => (open = false)} />

<button
  bind:this={trigger}
  class="select"
  class:open
  type="button"
  style:width={`${width}px`}
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  {disabled}
  onclick={() => (open ? hide() : show())}
  onkeydown={ontriggerkey}
>
  <span class="value" class:placeholder={!selected} title={selected?.label}>{selected?.label ?? placeholder}</span>
  <Icon name="chevron-down" size={15} />
</button>

{#if open}
  <div
    class="popover"
    class:up={place.up}
    bind:this={popover}
    style:left={`${place.left}px`}
    style:top={place.up ? undefined : `${place.top}px`}
    style:bottom={place.up ? `${window.innerHeight - place.top}px` : undefined}
    style:min-width={`${place.width}px`}
    style:max-width={`${Math.min(MAX_WIDTH, window.innerWidth - 2 * MARGIN)}px`}
  >
    {#if searchable}
      <input
        class="search"
        placeholder="Search"
        aria-label={`Search ${label}`}
        bind:value={query}
        oninput={() => (active = 0)}
        onkeydown={onlistkey}
      />
    {/if}
    <div
      class="list"
      role="listbox"
      tabindex="-1"
      aria-label={label}
      style:max-height={`${place.maxHeight - (searchable ? 44 : 0)}px`}
      bind:this={list}
      onkeydown={onlistkey}
    >
      {#each shown as option, i (option.value)}
        <button
          type="button"
          role="option"
          class="option"
          class:current={option.value === value}
          aria-selected={option.value === value}
          title={option.label}
          data-active={i === active}
          onclick={() => choose(option)}
          onpointermove={() => (active = i)}
        >
          <span class="check">{#if option.value === value}<Icon name="check" size={14} />{/if}</span>
          <span class="text">{option.label}</span>
          {#if option.hint}<span class="hint">{option.hint}</span>{/if}
        </button>
      {:else}
        <p class="none">Nothing matches</p>
      {/each}
    </div>
  </div>
{/if}

<style>
  .select {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 36px;
    padding: 0 10px 0 12px;
    border: none;
    border-radius: 10px;
    background: var(--fill);
    color: var(--text);
    text-align: left;
    transition: background 100ms, box-shadow 100ms;
  }

  .select:hover:not(:disabled) {
    background: var(--fill-hover);
  }

  .select:focus-visible,
  .select.open {
    box-shadow: 0 0 0 2px var(--accent);
    outline: none;
  }

  .select :global(svg) {
    flex-shrink: 0;
    color: var(--text-muted);
  }

  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value.placeholder {
    color: var(--text-muted);
  }

  .popover {
    position: fixed;
    z-index: 50;
    width: max-content;
    padding: 4px;
    border-radius: 12px;
    background: var(--popover);
    box-shadow: var(--shadow), 0 0 0 1px var(--line);
    animation: pop 110ms ease-out;
  }

  .popover.up {
    transform-origin: bottom;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.98);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .popover {
      animation: none;
    }
  }

  .search {
    width: 100%;
    height: 32px;
    margin-bottom: 4px;
    padding: 0 10px;
    border: none;
    border-radius: 8px;
    background: var(--fill);
    color: var(--text);
    font: inherit;
    outline: none;
  }

  .search:focus {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .list {
    overflow-y: auto;
    outline: none;
  }

  .option {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 34px;
    padding: 0 10px 0 6px;
    border: none;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  .option[data-active='true'] {
    background: var(--fill);
  }

  .option.current {
    font-weight: 500;
  }

  .check {
    width: 16px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    color: var(--accent);
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    flex-shrink: 0;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 400;
  }

  .none {
    margin: 0;
    padding: 10px;
    color: var(--text-muted);
  }
</style>
