// Laying the whole map out so related items form visible groups.
//
// A force layout started from scattered positions settles into one tangled ball: a few links
// between topics are enough to hold everything together. So the groups are found first
// (Louvain community detection over the weighted links), each starts in its own region, and
// ForceAtlas2 in LinLog mode, which pulls clusters apart far more than its default mode, then
// shapes them (in a worker, see `MapController.tidy`).

import type Graph from 'graphology'
import louvain from 'graphology-communities-louvain'
import forceAtlas2 from 'graphology-layout-forceatlas2'
import type { TopicsData } from '../types'

export type Point = { x: number; y: number }

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5))

/** The i-th point of a sunflower spiral with the given spacing. */
export function sunflower(index: number, spacing: number): Point {
  const r = spacing * Math.sqrt(index + 0.5)
  return { x: r * Math.cos(index * GOLDEN_ANGLE), y: r * Math.sin(index * GOLDEN_ANGLE) }
}

/** Starting positions with each group of related items on its own patch: the biggest group in
 * the middle, the rest spiralling outwards. */
export function groupedStart(graph: Graph, spacing: number): Record<string, Point> {
  if (graph.order === 0) return {}
  const community = louvain(graph, { getEdgeWeight: 'weight' })
  const groups = new Map<number, string[]>()
  graph.forEachNode((id) => {
    const members = groups.get(community[id]) ?? []
    members.push(id)
    groups.set(community[id], members)
  })
  const ordered = [...groups.values()].sort((a, b) => b.length - a.length)
  const patch = spacing * 3 * Math.sqrt(ordered[0].length)
  const positions: Record<string, Point> = {}
  ordered.forEach((members, g) => {
    const centre = sunflower(g, patch)
    members.forEach((id, i) => {
      const offset = sunflower(i, spacing)
      positions[id] = { x: centre.x + offset.x, y: centre.y + offset.y }
    })
  })
  return positions
}

/** Starting positions that follow the topic tree: each top-level topic gets a patch of the map,
 * its sub-topics lie inside it, and the items of the lowest topics gather in their own spots.
 * Returns null when there are no topics to follow. Items without a topic go in a patch of their
 * own. */
const UNTOPICED = '(items without a topic)'

export function topicStart(graph: Graph, data: TopicsData | null, spacing: number): Record<string, Point> | null {
  if (!data?.topics.length || graph.order === 0) return null
  const children = new Map<string | null, string[]>()
  for (const topic of data.topics) {
    const key = topic.parentId && data.topics.some((t) => t.id === topic.parentId) ? topic.parentId : null
    children.set(key, [...(children.get(key) ?? []), topic.id])
  }
  // Items in each lowest topic that are on the map.
  const own = new Map<string, string[]>()
  const placed = new Set<string>()
  for (const [item, topic] of Object.entries(data.itemTopic)) {
    if (!graph.hasNode(item)) continue
    own.set(topic, [...(own.get(topic) ?? []), item])
    placed.add(item)
  }
  const loose: string[] = []
  graph.forEachNode((id) => {
    if (!placed.has(id)) loose.push(id)
  })

  const size = new Map<string, number>()
  const sizeOf = (id: string): number => {
    const known = size.get(id)
    if (known !== undefined) return known
    const total = (own.get(id)?.length ?? 0) + (children.get(id) ?? []).reduce((sum, c) => sum + sizeOf(c), 0)
    size.set(id, total)
    return total
  }

  const positions: Record<string, Point> = {}
  const arrange = (ids: string[], sizes: number[], centre: Point, ring: number) => {
    // Biggest in the middle, the rest spiralling outwards.
    const order = ids.map((id, i) => ({ id, n: sizes[i] })).sort((a, b) => b.n - a.n)
    return order.map(({ id }, i) => ({ id, at: { x: centre.x + sunflower(i, ring).x, y: centre.y + sunflower(i, ring).y } }))
  }
  const lay = (id: string, centre: Point) => {
    const kids = (children.get(id) ?? []).filter((k) => sizeOf(k) > 0)
    if (!kids.length) {
      ;(own.get(id) ?? []).forEach((item, i) => {
        const offset = sunflower(i, spacing)
        positions[item] = { x: centre.x + offset.x, y: centre.y + offset.y }
      })
      return
    }
    const biggest = Math.max(...kids.map(sizeOf))
    const ring = spacing * 2.6 * Math.sqrt(biggest)
    for (const { id: kid, at } of arrange(kids, kids.map(sizeOf), centre, ring)) lay(kid, at)
  }

  const tops = (children.get(null) ?? []).filter((t) => sizeOf(t) > 0)
  const parts = [...tops, ...(loose.length ? [UNTOPICED] : [])]
  const biggest = Math.max(...tops.map(sizeOf), loose.length)
  const ring = spacing * 3 * Math.sqrt(biggest)
  const sizes = parts.map((id) => (id === UNTOPICED ? loose.length : sizeOf(id)))
  for (const { id, at } of arrange(parts, sizes, { x: 0, y: 0 }, ring)) {
    if (id === UNTOPICED) loose.forEach((item, i) => (positions[item] = { x: at.x + sunflower(i, spacing).x, y: at.y + sunflower(i, spacing).y }))
    else lay(id, at)
  }
  return positions
}

/** ForceAtlas2 settings that keep groups apart. Runs in a worker: it is quadratic in the number
 * of items without Barnes-Hut, too slow for the main thread past a few hundred. */
export function forceSettings(graph: Graph) {
  return {
    ...forceAtlas2.inferSettings(graph),
    linLogMode: true,
    barnesHutOptimize: graph.order > 400,
    // Unlinked items drift in from the edges rather than flying off.
    gravity: 0.5,
  }
}

/** How long the force layout runs: long enough to settle, scaled to the map's size. */
export function forceDuration(graph: Graph): number {
  return Math.min(5000, 1500 + graph.order * 2)
}

/** Positions scaled so the layout spans about as much as a spiral of the same nodes would,
 * keeping new items (placed at that scale) in proportion. The view frames the whole map, so
 * this doesn't visibly move anything. */
export function rescaled(graph: Graph, spacing: number): Record<string, Point> {
  let cx = 0
  let cy = 0
  graph.forEachNode((_, a) => {
    cx += a.x
    cy += a.y
  })
  cx /= graph.order
  cy /= graph.order
  const radii: number[] = []
  graph.forEachNode((_, a) => radii.push(Math.hypot(a.x - cx, a.y - cy)))
  radii.sort((a, b) => a - b)
  // The 90th percentile, so a few far-flung loners don't shrink everything else.
  const spread = radii[Math.floor(radii.length * 0.9)] || 1
  const scale = (spacing * Math.sqrt(graph.order)) / spread
  const positions: Record<string, Point> = {}
  graph.forEachNode((id, a) => (positions[id] = { x: (a.x - cx) * scale, y: (a.y - cy) * scale }))
  return positions
}
