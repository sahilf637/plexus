<script lang="ts">
  // The map's "View" menu: what the dots' colours and sizes mean, and how much stays bright around
  // the item you pick. Remembered on this computer (see lib/mapView.svelte.ts).
  import { AGE_COLORS } from '../lib/graph/palette'
  import { kindLabel } from '../lib/kinds'
  import { mapView, setMapView, VIEW_DEFAULT } from '../lib/mapView.svelte'
  import type { Kind } from '../lib/types'
  import Icon from './Icon.svelte'

  let open = $state(false)
  let trigger = $state<HTMLButtonElement>()
  let panel = $state<HTMLDivElement>()
  let place = $state({ right: 0, top: 0 })

  const kinds = Object.keys(kindLabel) as Kind[]
  const changed = $derived(
    mapView.colorBy !== VIEW_DEFAULT.colorBy ||
      mapView.sizeBy !== VIEW_DEFAULT.sizeBy ||
      mapView.reach !== VIEW_DEFAULT.reach ||
      mapView.hideRest !== VIEW_DEFAULT.hideRest,
  )
  const ageRamp = $derived.by(() => {
    const ends = window.matchMedia('(prefers-color-scheme: dark)').matches ? AGE_COLORS.dark : AGE_COLORS.light
    return `linear-gradient(to right, ${ends.old}, ${ends.recent})`
  })

  function toggle() {
    if (!open && trigger) {
      const rect = trigger.getBoundingClientRect()
      place = { right: Math.max(12, window.innerWidth - rect.right), top: rect.bottom + 8 }
    }
    open = !open
  }

  function outside(event: PointerEvent) {
    if (!open) return
    const target = event.target as Node
    if (!panel?.contains(target) && !trigger?.contains(target)) open = false
  }

  function onkeydown(event: KeyboardEvent) {
    if (open && event.key === 'Escape') {
      event.stopPropagation()
      open = false
      trigger?.focus()
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydowncapture={onkeydown} />

<button
  bind:this={trigger}
  class="icon-btn"
  class:on={open || changed}
  onclick={toggle}
  title="View: colours, sizes and focus"
  aria-label="View"
  aria-haspopup="dialog"
  aria-expanded={open}
>
  <Icon name="eye" />
</button>

{#if open}
  <div class="view" bind:this={panel} role="dialog" aria-label="View" style:right={`${place.right}px`} style:top={`${place.top}px`}>
    <section>
      <h4>Colour by</h4>
      <div class="segmented" role="radiogroup" aria-label="Colour by">
        {#each [['type', 'Type'], ['topic', 'Topic'], ['age', 'Age']] as [value, label] (value)}
          <button role="radio" aria-checked={mapView.colorBy === value} class:on={mapView.colorBy === value} onclick={() => setMapView({ colorBy: value as typeof mapView.colorBy })}>{label}</button>
        {/each}
      </div>
      <div class="legend">
        {#if mapView.colorBy === 'type'}
          {#each kinds as kind (kind)}
            <span><i style:background={`var(--kind-${kind})`}></i>{kindLabel[kind]}</span>
          {/each}
        {:else if mapView.colorBy === 'topic'}
          <span>Matches the topic regions</span>
        {:else}
          <span class="ramp" style:background={ageRamp}></span>
          <span class="ends"><span>Older</span><span>Newer</span></span>
        {/if}
      </div>
    </section>

    <section>
      <h4>Size by</h4>
      <div class="segmented" role="radiogroup" aria-label="Size by">
        {#each [['links', 'Links'], ['same', 'Same']] as [value, label] (value)}
          <button role="radio" aria-checked={mapView.sizeBy === value} class:on={mapView.sizeBy === value} onclick={() => setMapView({ sizeBy: value as typeof mapView.sizeBy })}>{label}</button>
        {/each}
      </div>
      <p class="hint">{mapView.sizeBy === 'links' ? 'More links, bigger dot' : 'One size for every dot'}</p>
    </section>

    <section>
      <h4>When you pick or hover a note</h4>
      <div class="segmented" role="radiogroup" aria-label="Reach">
        {#each [[1, 'Direct links'], [2, 'Two steps']] as [value, label] (value)}
          <button role="radio" aria-checked={mapView.reach === value} class:on={mapView.reach === value} onclick={() => setMapView({ reach: value as 1 | 2 })}>{label}</button>
        {/each}
      </div>
      <label class="row">
        <span>Hide the rest<span class="hint">Only for the picked note</span></span>
        <input type="checkbox" class="switch" checked={mapView.hideRest} onchange={(e) => setMapView({ hideRest: e.currentTarget.checked })} />
      </label>
    </section>

    {#if changed}
      <button class="btn reset" onclick={() => setMapView({ ...VIEW_DEFAULT })}>Reset</button>
    {/if}
  </div>
{/if}

<style>
  .icon-btn.on {
    background: var(--fill);
    color: var(--text);
  }

  .view {
    position: fixed;
    z-index: 30;
    width: 300px;
    max-width: calc(100vw - 24px);
    padding: 12px 14px 14px;
    border-radius: 12px;
    background: var(--popover);
    box-shadow: var(--shadow), 0 0 0 1px var(--line);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .segmented {
    display: flex;
  }

  .segmented button {
    flex: 1;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  h4 {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .legend > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .ramp {
    width: 100%;
    height: 8px;
    border-radius: 4px;
  }

  .ends {
    width: 100%;
    display: flex !important;
    justify-content: space-between;
  }

  .hint {
    display: block;
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 400;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .reset {
    align-self: flex-end;
  }
</style>
