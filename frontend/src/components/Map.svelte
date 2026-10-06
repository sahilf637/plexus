<script lang="ts">
  import { onMount } from 'svelte'
  import { MapController, type PanDirection } from '../lib/graph/map'
  import type { PathState } from '../lib/graph/path'
  import { navigation } from '../lib/navigation.svelte'
  import type { Edge, Item, MapData, Position, Tier, TopicsData } from '../lib/types'

  let {
    data,
    selected,
    matches,
    lowestTier,
    marked,
    topics,
    selectedTopic,
    inset,
    askPath,
    onselect,
    onmenu,
    onplaced,
    ontopic,
  }: {
    data: MapData
    selected: string | null
    matches: string[] | null
    lowestTier: Tier
    marked: string[]
    /** The topic tree, drawn as regions and names that change with the zoom. */
    topics: TopicsData
    selectedTopic: string | null
    /** Width of the panel docked over the map's right edge (0 when there is none). */
    inset: number
    /** Where the Ask agent has been looking, drawn over the map while the Ask panel is showing. */
    askPath: PathState
    ontopic: (id: string) => void
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
      topic: (id) => ontopic(id),
      select: (id, additive = false) => onselect(id, additive),
      menu: (id, x, y) => onmenu(id, x, y),
      placed: (p) => onplaced(p),
    })
    map.setTopics($state.snapshot(topics))
    map.load($state.snapshot(data))
    return () => map?.destroy()
  })

  // Before the effects that move the view, so they centre in what the panel leaves.
  $effect(() => {
    map?.setInset(inset)
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
    map?.setTopics($state.snapshot(topics))
  })

  $effect(() => {
    map?.setSelectedTopic(selectedTopic)
  })

  $effect(() => {
    map?.setAskPath($state.snapshot(askPath))
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
