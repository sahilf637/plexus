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
