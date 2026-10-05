<script lang="ts">
  import { onMount } from 'svelte'
  import { MapController, type PanDirection } from '../lib/graph/map'
  import { navigation } from '../lib/navigation.svelte'
  import type { Edge, Item, MapData, Position, Tier } from '../lib/types'

  let {
    data,
    selected,
    matches,
    lowestTier,
    marked,
    onselect,
    onmenu,
    onplaced,
  }: {
    data: MapData
    selected: string | null
    matches: string[] | null
    lowestTier: Tier
    marked: string[]
    onselect: (id: string | null, additive: boolean) => void
    onmenu: (id: string, x: number, y: number) => void
    onplaced: (positions: Position[]) => void
  } = $props()

  let container = $state<HTMLDivElement>()
  let map: MapController | undefined

  export function addItem(item: Item) {
    map?.addItem(item)
  }

  export function removeItems(ids: string[]) {
    map?.removeItems(ids)
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

  export function panKey(direction: PanDirection, down: boolean, fast = false) {
    map?.panKey(direction, down, fast)
  }

  export function stopPan() {
    map?.stopPan()
  }

  export function zoom(inwards: boolean) {
    map?.zoom(inwards)
  }

  export function showAll() {
    map?.showAll()
  }

  export function showSelected() {
    map?.showSelected()
  }

  onMount(() => {
    map = new MapController(container!, {
      select: (id, additive = false) => onselect(id, additive),
      menu: (id, x, y) => onmenu(id, x, y),
      placed: (p) => onplaced(p),
    })
    map.load($state.snapshot(data))
    return () => map?.destroy()
  })

  $effect(() => {
    map?.select(selected)
  })

  $effect(() => {
    map?.setMarked(marked)
  })

  $effect(() => {
    map?.setMatches(matches)
  })

  $effect(() => {
    map?.setLowestTier(lowestTier)
  })

  $effect(() => {
    map?.setNavigation({ ...navigation })
  })
</script>

<div class="map" bind:this={container}></div>

<style>
  .map {
    position: absolute;
    inset: 0;
  }
</style>
