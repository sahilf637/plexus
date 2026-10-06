<script lang="ts">
  // A picture full size over the whole window: scroll or the buttons zoom, dragging moves it,
  // a double click switches between fitted and actual size, Esc or a click outside closes it.
  import { navigation } from '../lib/navigation.svelte'
  import Icon from './Icon.svelte'

  let { src, alt, onclose }: { src: string; alt: string; onclose: () => void } = $props()

  const MIN = 0.25
  const MAX = 8
  let zoom = $state(1)
  let x = $state(0)
  let y = $state(0)
  let drag: { x: number; y: number; from: { x: number; y: number } } | null = null
  let moved = false
  /** The press started on the dark area around the picture (so letting go there closes). */
  let onBackdrop = false
  let img = $state<HTMLImageElement>()
  /** Where the drag has got to, applied once per frame rather than once per mouse event. */
  let pending: { x: number; y: number } | null = null

  function zoomBy(factor: number, at?: { x: number; y: number }) {
    const next = Math.min(MAX, Math.max(MIN, zoom * factor))
    // Keep the point under the cursor where it is.
    if (at) {
      x = at.x - ((at.x - x) * next) / zoom
      y = at.y - ((at.y - y) * next) / zoom
    }
    zoom = next
  }

  function reset() {
    zoom = 1
    x = 0
    y = 0
  }

  /** Fitted ↔ the picture's own pixels. */
  function toggleActual(event: MouseEvent) {
    if (!img || (event.target as Element).closest('.tools')) return
    const actual = img.naturalWidth / img.clientWidth
    if (Math.abs(zoom - 1) > 0.01 || actual <= 1.05) return reset()
    zoomBy(actual, fromCentre(event))
  }

  /** A pointer position relative to the middle of the window, where the picture is centred. */
  const fromCentre = (event: MouseEvent) => ({ x: event.clientX - window.innerWidth / 2, y: event.clientY - window.innerHeight / 2 })

  function onwheel(event: WheelEvent) {
    event.preventDefault()
    // The same direction as on the map (the "Reverse zoom direction" setting).
    const direction = navigation.invertZoom ? 1 : -1
    zoomBy(Math.exp(direction * event.deltaY * 0.0025), fromCentre(event))
  }

  function onkeydown(event: KeyboardEvent) {
    // The map listens for Esc and + / - too; the lightbox has them first.
    if (['Escape', '+', '=', '-', '_', '0'].includes(event.key)) {
      event.preventDefault()
      event.stopImmediatePropagation()
    }
    if (event.key === 'Escape') onclose()
    else if (event.key === '+' || event.key === '=') zoomBy(1.25)
    else if (event.key === '-' || event.key === '_') zoomBy(0.8)
    else if (event.key === '0') reset()
  }
</script>

<svelte:window onkeydowncapture={onkeydown} />

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events -->
<div
  class="lightbox"
  role="dialog"
  aria-modal="true"
  aria-label={alt}
  tabindex="-1"
  {onwheel}
  ondblclick={toggleActual}
  onpointerdown={(event) => {
    onBackdrop = event.target === event.currentTarget
    if (event.button !== 0 || (event.target as Element).closest('.tools')) return
    moved = false
    drag = { x: event.clientX, y: event.clientY, from: { x, y } }
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  }}
  onpointermove={(event) => {
    if (!drag) return
    const dx = event.clientX - drag.x
    const dy = event.clientY - drag.y
    if (Math.abs(dx) + Math.abs(dy) > 3) moved = true
    const first = !pending
    pending = { x: drag.from.x + dx, y: drag.from.y + dy }
    if (first)
      requestAnimationFrame(() => {
        if (pending) ({ x, y } = pending)
        pending = null
      })
  }}
  onpointerup={() => (drag = null)}
  onclick={(event) => {
    // A click on the dark area closes; a drag that ends there doesn't.
    if (!moved && onBackdrop && event.detail === 1) onclose()
  }}
>
  <img bind:this={img} {src} {alt} draggable="false" decoding="async" style:transform={`translate(${x}px, ${y}px) scale(${zoom})`} />

  <div class="tools">
    <button class="icon-btn" onclick={() => zoomBy(0.8)} title="Zoom out (-)" aria-label="Zoom out"><Icon name="zoom-out" /></button>
    <button class="pct" onclick={reset} title="Fit (0)">{Math.round(zoom * 100)}%</button>
    <button class="icon-btn" onclick={() => zoomBy(1.25)} title="Zoom in (+)" aria-label="Zoom in"><Icon name="zoom-in" /></button>
    <button class="icon-btn" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
  </div>
</div>

<style>
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    overflow: hidden;
    background: rgb(0 0 0 / 0.82);
    cursor: grab;
    animation: fade 150ms ease-out;
    outline: none;
  }

  .lightbox:active {
    cursor: grabbing;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .lightbox {
      animation: none;
    }
  }

  img {
    max-width: calc(100vw - 64px);
    max-height: calc(100vh - 96px);
    border-radius: 4px;
    transform-origin: center;
    /* Its own layer, so moving and zooming it doesn't repaint the picture (a shadow around it
       would be repainted every frame). */
    will-change: transform;
    user-select: none;
  }

  .tools {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 12px;
    background: rgb(30 30 36 / 0.9);
    color: white;
    cursor: default;
  }

  .tools .icon-btn {
    color: rgb(255 255 255 / 0.75);
  }

  .tools .icon-btn:hover {
    background: rgb(255 255 255 / 0.12);
    color: white;
  }

  .pct {
    min-width: 54px;
    height: 30px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: rgb(255 255 255 / 0.85);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .pct:hover {
    background: rgb(255 255 255 / 0.12);
  }
</style>
