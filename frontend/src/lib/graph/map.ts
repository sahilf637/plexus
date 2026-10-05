// The map: a Sigma.js (WebGL) view over a Graphology graph of items. The Svelte component owns
// one of these; everything about drawing and interaction lives here.

import Graph from 'graphology'
import Sigma from 'sigma'
import EdgeCurveProgram from '@sigma/edge-curve'
import type { EdgeDisplayData, NodeDisplayData, PartialButFor } from 'sigma/types'
import type { Settings } from 'sigma/settings'
import type { Edge, Item, MapData, Position } from '../types'

export interface MapEvents {
  select: (id: string | null) => void
  /** Positions decided by the map (new items), to be saved. */
  placed: (positions: Position[]) => void
}

interface NodeAttributes {
  x: number
  y: number
  size: number
  label: string
  color: string
  kind: Item['kind']
  highlighted?: boolean
}

const LABEL_MAX = 36
const PULSE_MS = 2500
/** New items without links are laid out on a sunflower spiral, in the order they arrived. */
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5))
const SPIRAL_SPACING = 10

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

function themeColors() {
  return {
    text: cssVar('--kind-text'),
    code: cssVar('--kind-code'),
    url: cssVar('--kind-url'),
    image: cssVar('--kind-image'),
    file: cssVar('--kind-file'),
    dim: cssVar('--map-dim'),
    edge: cssVar('--map-edge'),
    label: cssVar('--text'),
    surface: cssVar('--surface'),
    border: cssVar('--border'),
  }
}

function shortLabel(title: string): string {
  return title.length > LABEL_MAX ? `${title.slice(0, LABEL_MAX - 1).trimEnd()}…` : title
}

export function spiralPosition(index: number): { x: number; y: number } {
  const r = SPIRAL_SPACING * Math.sqrt(index + 0.5)
  return { x: r * Math.cos(index * GOLDEN_ANGLE), y: r * Math.sin(index * GOLDEN_ANGLE) }
}

export class MapController {
  private graph = new Graph<NodeAttributes>({ type: 'undirected' })
  private sigma: Sigma
  private colors = themeColors()
  private hovered: string | null = null
  private selected: string | null = null
  private matches: Set<string> | null = null
  private events: MapEvents
  private themeQuery = window.matchMedia('(prefers-color-scheme: dark)')

  constructor(container: HTMLElement, events: MapEvents) {
    this.events = events
    // Sigma's types don't carry our node attributes through; the reducers cast back.
    this.sigma = new Sigma(this.graph as unknown as Graph, container, {
      defaultEdgeType: 'curve',
      edgeProgramClasses: { curve: EdgeCurveProgram },
      labelFont: 'system-ui, sans-serif',
      labelSize: 12,
      labelWeight: '500',
      labelColor: { color: this.colors.label },
      defaultDrawNodeHover: (context, data, settings) => this.drawHover(context, data, settings),
      // Every label may show; Sigma's label grid drops the ones that would overlap.
      labelRenderedSizeThreshold: 0,
      labelDensity: 0.8,
      labelGridCellSize: 110,
      zIndex: true,
      minCameraRatio: 0.02,
      maxCameraRatio: 20,
      nodeReducer: (node, data) => this.reduceNode(node, data as unknown as NodeAttributes),
      edgeReducer: (edge, data) => this.reduceEdge(edge, data as unknown as EdgeDisplayData),
    })

    this.sigma.on('enterNode', ({ node }) => {
      this.hovered = node
      container.style.cursor = 'pointer'
      this.sigma.refresh({ skipIndexation: true })
    })
    this.sigma.on('leaveNode', () => {
      this.hovered = null
      container.style.cursor = ''
      this.sigma.refresh({ skipIndexation: true })
    })
    this.sigma.on('clickNode', ({ node }) => this.events.select(node))
    this.sigma.on('clickStage', () => this.events.select(null))
    this.themeQuery.addEventListener('change', this.onThemeChange)
  }

  /** Replaces everything on the map. */
  load(data: MapData) {
    this.graph.clear()
    const saved = new Map(data.positions.map((p) => [p.id, p]))
    const placed: Position[] = []
    data.items.forEach((item, index) => {
      const position = saved.get(item.id) ?? spiralPosition(index)
      if (!saved.has(item.id)) placed.push({ id: item.id, ...position })
      this.graph.addNode(item.id, this.attributes(item, position))
    })
    this.setEdges(data.edges)
    if (placed.length) this.events.placed(placed)
  }

  /** A new item arrives: it appears straight away, pulses, and the camera brings it into view. */
  addItem(item: Item) {
    if (this.graph.hasNode(item.id)) {
      this.graph.mergeNodeAttributes(item.id, { label: shortLabel(item.title), kind: item.kind })
      return
    }
    const position = spiralPosition(this.graph.order)
    this.graph.addNode(item.id, { ...this.attributes(item, position), highlighted: true })
    this.events.placed([{ id: item.id, ...position }])
    setTimeout(() => {
      if (this.graph.hasNode(item.id)) this.graph.setNodeAttribute(item.id, 'highlighted', false)
    }, PULSE_MS)
    // Sigma only knows where the node is on screen once it has drawn it: draw now, then bring
    // it into view if it landed off-screen.
    this.sigma.refresh()
    if (!this.isOnScreen(item.id)) this.focus(item.id, false)
  }

  setEdges(edges: Edge[]) {
    this.graph.clearEdges()
    for (const edge of edges) {
      if (!this.graph.hasNode(edge.src) || !this.graph.hasNode(edge.dst)) continue
      if (this.graph.hasEdge(edge.src, edge.dst)) continue
      this.graph.addEdge(edge.src, edge.dst, { size: 0.5 + 2 * edge.weight, color: this.colors.edge })
    }
    this.graph.forEachNode((node) => this.graph.setNodeAttribute(node, 'size', nodeSize(this.graph.degree(node))))
  }

  select(id: string | null) {
    this.selected = id
    this.sigma.refresh({ skipIndexation: true })
    if (id) this.focus(id, true)
  }

  /** Search results: everything else is dimmed. `null` clears the search. */
  setMatches(ids: string[] | null) {
    this.matches = ids ? new Set(ids) : null
    this.sigma.refresh({ skipIndexation: true })
    if (ids?.length) this.fit(ids)
  }

  /** Current positions, for saving. */
  positions(): Position[] {
    return this.graph.mapNodes((id, attrs) => ({ id, x: attrs.x, y: attrs.y }))
  }

  destroy() {
    this.themeQuery.removeEventListener('change', this.onThemeChange)
    this.sigma.kill()
  }

  private isOnScreen(id: string, margin = 10): boolean {
    const display = this.sigma.getNodeDisplayData(id)
    if (!display) return false
    const { x, y } = this.sigma.framedGraphToViewport(display)
    const { width, height } = this.sigma.getDimensions()
    return x >= margin && y >= margin && x <= width - margin && y <= height - margin
  }

  /** Moves the camera so all of these nodes are in view. */
  private fit(ids: string[]) {
    const points = ids.map((id) => this.sigma.getNodeDisplayData(id)).filter((d) => d !== undefined)
    if (!points.length) return
    const xs = points.map((p) => p.x)
    const ys = points.map((p) => p.y)
    const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)]
    // Camera ratio 1 shows the whole map; leave a margin around the matches.
    const ratio = Math.min(1, Math.max(0.15, Math.max(maxX - minX, maxY - minY) * 1.4))
    this.sigma.getCamera().animate({ x: (minX + maxX) / 2, y: (minY + maxY) / 2, ratio }, { duration: 450 })
  }

  /** Moves the camera to a node; `zoomIn` also zooms in if we're far out. */
  private focus(id: string, zoomIn: boolean) {
    const display = this.sigma.getNodeDisplayData(id)
    if (!display) return
    const camera = this.sigma.getCamera()
    const ratio = zoomIn ? Math.min(camera.ratio, 0.5) : camera.ratio
    camera.animate({ x: display.x, y: display.y, ratio }, { duration: 450 })
  }

  private attributes(item: Item, position: { x: number; y: number }): NodeAttributes {
    return {
      ...position,
      size: nodeSize(0),
      label: shortLabel(item.title),
      color: this.colors[item.kind],
      kind: item.kind,
    }
  }

  private reduceNode(node: string, data: NodeAttributes): Partial<NodeDisplayData> {
    const result: Partial<NodeDisplayData> = { ...data }
    const outsideSearch = this.matches !== null && !this.matches.has(node)
    const outsideHover =
      this.hovered !== null && node !== this.hovered && !this.graph.areNeighbors(node, this.hovered)
    if (outsideSearch || outsideHover) {
      result.color = this.colors.dim
      result.label = ''
      result.zIndex = 0
    } else {
      result.zIndex = 1
    }
    if (node === this.selected || node === this.hovered) {
      result.highlighted = true
      result.forceLabel = true
      result.zIndex = 2
    }
    return result
  }

  private reduceEdge(edge: string, data: EdgeDisplayData): Partial<EdgeDisplayData> {
    const [a, b] = this.graph.extremities(edge)
    if (this.hovered && a !== this.hovered && b !== this.hovered) return { ...data, hidden: true }
    if (this.matches && !(this.matches.has(a) && this.matches.has(b))) return { ...data, hidden: true }
    return data
  }

  /** The label box for hovered and highlighted nodes, in the app's colours (Sigma's default is
   * a white box, unreadable in dark mode). */
  private drawHover(context: CanvasRenderingContext2D, data: PartialButFor<NodeDisplayData, 'x' | 'y' | 'size' | 'label' | 'color'>, settings: Settings) {
    const size = settings.labelSize
    context.font = `${settings.labelWeight} ${size}px ${settings.labelFont}`
    const label = data.label ?? ''
    const padding = 4
    const width = context.measureText(label).width + data.size + padding * 3
    const height = size + padding * 2
    const x = data.x - data.size - padding
    const y = data.y - height / 2

    context.fillStyle = this.colors.surface
    context.strokeStyle = this.colors.border
    context.lineWidth = 1
    context.beginPath()
    context.roundRect(x, y, width + data.size, height, 6)
    context.fill()
    context.stroke()

    context.fillStyle = data.color
    context.beginPath()
    context.arc(data.x, data.y, data.size, 0, Math.PI * 2)
    context.fill()

    context.fillStyle = this.colors.label
    context.fillText(label, data.x + data.size + padding, data.y + size / 3)
  }

  private onThemeChange = () => {
    this.colors = themeColors()
    this.sigma.setSetting('labelColor', { color: this.colors.label })
    this.graph.forEachNode((node, attrs) => this.graph.setNodeAttribute(node, 'color', this.colors[attrs.kind]))
    this.graph.forEachEdge((edge) => this.graph.setEdgeAttribute(edge, 'color', this.colors.edge))
  }
}

/** Better-connected items are drawn bigger. */
function nodeSize(degree: number): number {
  return 5 + 1.6 * Math.sqrt(degree)
}
