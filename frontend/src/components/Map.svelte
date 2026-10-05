<script lang="ts">
  import { onMount } from 'svelte'
  import { MapController } from '../lib/graph/map'
  import type { Edge, Item, MapData, Position, Tier } from '../lib/types'

  let {
    data,
    selected,
    matches,
    lowestTier,
    onselect,
    onplaced,
  }: {
    data: MapData
    selected: string | null
    matches: string[] | null
    lowestTier: Tier
    onselect: (id: string | null) => void
    onplaced: (positions: Position[]) => void
  } = $props()

  let container = $state<HTMLDivElement>()
  let map: MapController | undefined

  export function addItem(item: Item) {
    map?.addItem(item)
  }

  export function setEdges(edges: Edge[]) {
    map?.setEdges(edges)
  }

  export function settle(id: string) {
    map?.settle(id)
  }

  export function tidy(): Promise<void> {
    return map?.tidy() ?? Promise.resolve()
  }

  onMount(() => {
    map = new MapController(container!, { select: (id) => onselect(id), placed: (p) => onplaced(p) })
    map.load($state.snapshot(data))
    return () => map?.destroy()
  })

  $effect(() => {
    map?.select(selected)
  })

  $effect(() => {
    map?.setMatches(matches)
  })

  $effect(() => {
    map?.setLowestTier(lowestTier)
  })
</script>

<div class="map" bind:this={container}></div>

<style>
  .map {
    position: absolute;
    inset: 0;
  }
</style>
