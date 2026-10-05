// The map: a Sigma.js (WebGL) view over a Graphology graph of items. The Svelte component owns
// one of these; everything about drawing and interaction lives here.

import Graph from 'graphology'
import Sigma from 'sigma'
import EdgeCurveProgram from '@sigma/edge-curve'
import FA2Layout from 'graphology-layout-forceatlas2/worker'
import { animateNodes } from 'sigma/utils'
import type { EdgeDisplayData, NodeDisplayData, PartialButFor } from 'sigma/types'
import type { Settings } from 'sigma/settings'
import { tierShown } from '../kinds'
import type { Navigation } from '../navigation.svelte'
import { forceDuration, forceSettings, groupedStart, rescaled, sunflower } from './layout'
import type { Edge, Item, MapData, Position, Tier } from '../types'

/** How each tier of link is drawn, and how hard it pulls its items together in the layout. */
const TIER_STYLE: Record<Tier, { size: number; weight: number }> = {
  strong: { size: 2.4, weight: 1 },
  related: { size: 1.2, weight: 0.5 },
  maybe: { size: 0.7, weight: 0.12 },
}

export interface MapEvents {
  /** A click on an item (`additive`: with Shift, to build a selection) or on empty space. */
  select: (id: string | null, additive?: boolean) => void
  /** Right-click on an item, at these pixel coordinates in the map. */
  menu: (id: string, x: number, y: number) => void
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
const SPIRAL_SPACING = 10
const SETTLE_MS = 600
/** Items move to their group's patch before the force layout takes over. */
const GROUP_MS = 500
/** This many items settling in a burst (an import, the first start on a library) gets the whole
 * map tidied once the burst is over, since one-by-one placement can't form groups at that scale. */
const BURST_SIZE = 15
const BURST_QUIET_MS = 2500
/** How long the gentle relayout runs after a few items settled. */
const RELAX_MS = 1200

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
    edgeStrong: cssVar('--map-edge-strong'),
    edgeMaybe: cssVar('--map-edge-maybe'),
    label: cssVar('--text'),
    surface: cssVar('--surface'),
    border: cssVar('--border'),
  }
}

function shortLabel(title: string): string {
  return title.length > LABEL_MAX ? `${title.slice(0, LABEL_MAX - 1).trimEnd()}…` : title
}

export function spiralPosition(index: number): { x: number; y: number } {
  return sunflower(index, SPIRAL_SPACING)
}

export type PanDirection = 'left' | 'right' | 'up' | 'down'

/** How far the view glides per second while an arrow key is held, in screens (Shift: faster). */
const PAN_SPEED = 0.9
const PAN_SPEED_FAST = 2.4

const MIN_RATIO = 0.02
/** Ratio 1 shows the whole map; zooming out stops a little past that, so the view can't drift
 * off into empty space or onto far-flung loners. */
const MAX_RATIO = 2.5
/** Zoom per pixel of scrolling: a mouse-wheel notch (about 100) zooms by roughly 1.4 times, a
 * trackpad's many small events add up to the same; pinching (sent with Ctrl held) is finer. */
const WHEEL_SPEED = 0.0034
const PINCH_SPEED = 0.01
/** Following the cursor: the view drifts towards it, at a speed that grows exponentially with
 * the cursor's distance from the middle (near nothing around the centre, `FOLLOW_SPEED` screens
 * per second at the very edge). `FOLLOW_CURVE` is how steeply it picks up. */
const FOLLOW_SPEED = 0.8
const FOLLOW_CURVE = 5
/** Below this fraction of the top speed the view stays still (and the frame loop rests). */
const FOLLOW_MIN = 0.008
/** How far past the outermost items the view may go (in view units; the whole map is about 1). */
const EDGE_ROOM = 0.08

/** Magnetic hover: the cursor counts as touching the nearest item when it is within the dot plus
 * this many pixels, and clearly nearer to it than to the runner-up (closer than this fraction of
 * the runner-up's distance). That is what keeps the reach small where items crowd together and
 * generous where one stands alone. The item already touched is held on to more firmly. */
const MAGNET_REACH = 10
const MAGNET_CLEAR = 0.6
const MAGNET_STICKY_REACH = 15
const MAGNET_STICKY_CLEAR = 0.85

/** One scroll event never counts for more than this many pixels, so a burst can't leap. */
const MAX_PIXELS = 60
/** How quickly the view catches up with the zoom asked for (time to cover ~63% of the gap). */
const ZOOM_EASE_MS = 70

export class MapController {
  private graph = new Graph<NodeAttributes>({ type: 'undirected' })
  private sigma: Sigma
  private colors = themeColors()
  private hovered: string | null = null
  /** The least sure tier of link drawn (hovering an item shows all of its links). */
  private lowestTier: Tier = 'related'
  private burst = 0
  private layout: FA2Layout | undefined
  private burstTimer: ReturnType<typeof setTimeout> | undefined
  private container: HTMLElement
  private navigation: Navigation = { invertZoom: true, edgeScroll: true }
  /** Where the cursor is on the map (in pixels), or null when it isn't over it. */
  private pointer: { x: number; y: number } | null = null
  private edgeFrame = 0
  private dragging = false
  /** The map's extent in view coordinates, measured when a glide starts; the view stays inside. */
  private bounds = { minX: 0, maxX: 1, minY: 0, maxY: 1 }
  private edgeLast = 0
  /** The zoom scrolling has asked for; the view eases towards it a frame at a time. */
  private zoomAim: { ratio: number; cursor: { x: number; y: number } } | null = null
  private zoomFrame = 0
  private zoomLast = 0
  private panHeld = new Set<PanDirection>()
  private panFast = false
  private panFrame = 0
  private panLast = 0
  private selected: string | null = null
  /** Items picked together (Shift-click), e.g. to delete them in one go. */
  private marked = new Set<string>()
  private matches: Set<string> | null = null
  private events: MapEvents
  private themeQuery = window.matchMedia('(prefers-color-scheme: dark)')
  /** The window can start hidden (launched at login), with no size yet: redraw once it has one. */
  private resizeObserver = new ResizeObserver(() => this.sigma.refresh())

  constructor(container: HTMLElement, events: MapEvents) {
    this.events = events
    this.container = container
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
      allowInvalidContainer: true,
      minCameraRatio: MIN_RATIO,
      maxCameraRatio: MAX_RATIO,
      nodeReducer: (node, data) => this.reduceNode(node, data as unknown as NodeAttributes),
      edgeReducer: (edge, data) => this.reduceEdge(edge, data as unknown as EdgeDisplayData),
    })

    // Which item the cursor touches is worked out in `retarget` (it reaches a little past the
    // dot), so Sigma's own enter/leave events are not used; clicks go to that item too.
    this.sigma.on('clickNode', ({ node, event }) => this.click(node, event))
    this.sigma.on('clickStage', ({ event }) => this.click(null, event))
    this.sigma.on('rightClickNode', ({ node, event }) => {
      event.original.preventDefault()
      this.events.menu(this.touched(event, node) ?? node, event.x, event.y)
    })
    this.sigma.on('rightClickStage', ({ event }) => {
      // The webview's own menu (Reload, Inspect…) has no place on the map.
      event.original.preventDefault()
      const target = this.touched(event, null)
      if (target) this.events.menu(target, event.x, event.y)
    })
    this.themeQuery.addEventListener('change', this.onThemeChange)
    this.resizeObserver.observe(container)
    // Captured, so it runs before Sigma's own wheel zoom, which it replaces.
    container.addEventListener('wheel', this.onwheel, { passive: false, capture: true })
    container.addEventListener('pointermove', this.onpointermove)
    container.addEventListener('pointerleave', this.onpointerleave)
    window.addEventListener('blur', this.onpointerleave)
    window.addEventListener('pointerup', this.onpointerup)
  }

  /** Notes where the items are, so a glide can stop at the map's edge instead of drifting into
   * empty space. */
  private measure() {
    const points = this.graph.nodes().map((id) => this.sigma.getNodeDisplayData(id)).filter((d) => d !== undefined)
    if (!points.length) return
    const xs = points.map((p) => p.x)
    const ys = points.map((p) => p.y)
    this.bounds = { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) }
  }

  /** The view's centre, held so the view never goes past the map's extent plus a little room.
   * When the whole map already fits on screen along an axis, the view doesn't move that way. */
  private inside(point: { x: number; y: number }): { x: number; y: number } {
    const { minX, maxX, minY, maxY } = this.bounds
    const { width, height } = this.sigma.getDimensions()
    const ratio = this.sigma.getCamera().ratio
    // The view spans about `ratio` view units along its longer side.
    const halfX = (ratio / 2) * (width / Math.max(width, height))
    const halfY = (ratio / 2) * (height / Math.max(width, height))
    const camera = this.sigma.getCamera()
    const hold = (value: number, current: number, min: number, max: number, half: number) => {
      const low = min - EDGE_ROOM + half
      const high = max + EDGE_ROOM - half
      return low > high ? current : Math.min(high, Math.max(low, value))
    }
    return { x: hold(point.x, camera.x, minX, maxX, halfX), y: hold(point.y, camera.y, minY, maxY, halfY) }
  }

  private onpointerup = () => {
    this.dragging = false
  }

  private onpointermove = (event: PointerEvent) => {
    const rect = this.container.getBoundingClientRect()
    this.pointer = { x: event.clientX - rect.left, y: event.clientY - rect.top }
    this.dragging = event.buttons !== 0
    this.retarget()
    if (this.navigation.edgeScroll && !this.edgeFrame) {
      this.measure()
      this.edgeLast = performance.now()
      this.edgeFrame = requestAnimationFrame(this.edgeStep)
    }
  }

  private onpointerleave = () => {
    this.pointer = null
    this.retarget()
  }

  /** The item a click or right-click is for: the one already shown as touched, else whatever
   * is within reach of where the click landed (so it doesn't depend on a move before it). */
  private touched(at: { x: number; y: number }, node: string | null): string | null {
    return this.hovered ?? this.itemAt(at) ?? node
  }

  private click(node: string | null, event: { x: number; y: number; original: MouseEvent | TouchEvent }) {
    this.events.select(this.touched(event, node), 'shiftKey' in event.original && event.original.shiftKey)
  }

  /** The item the cursor counts as touching, if any; see `MAGNET_REACH`. */
  private itemAt(pointer: { x: number; y: number }): string | null {
    let nearest: string | null = null
    let nearestDistance = Infinity
    let nearestRadius = 0
    let runnerUp = Infinity
    this.graph.forEachNode((id) => {
      const display = this.sigma.getNodeDisplayData(id)
      if (!display || display.hidden) return
      const at = this.sigma.framedGraphToViewport(display)
      const distance = Math.hypot(at.x - pointer.x, at.y - pointer.y)
      if (distance < nearestDistance) {
        runnerUp = nearestDistance
        nearest = id
        nearestDistance = distance
        nearestRadius = this.sigma.scaleSize(display.size)
      } else if (distance < runnerUp) {
        runnerUp = distance
      }
    })
    if (nearest === null) return null
    const radius = Math.max(nearestRadius, 3)
    // On the dot itself: always that item.
    if (nearestDistance <= radius) return nearest
    const held = nearest === this.hovered
    const reach = radius + (held ? MAGNET_STICKY_REACH : MAGNET_REACH)
    const clear = nearestDistance < (held ? MAGNET_STICKY_CLEAR : MAGNET_CLEAR) * runnerUp
    return nearestDistance <= reach && clear ? nearest : null
  }

  /** Works out which item is touched now (the pointer moved, or the map moved under it). */
  private retarget() {
    const found = this.pointer ? this.itemAt(this.pointer) : null
    if (found === this.hovered) return
    this.hovered = found
    this.container.style.cursor = found ? 'pointer' : ''
    this.sigma.refresh({ skipIndexation: true })
  }

  /** The speed, as a fraction of the top one, for a cursor at `position` along a side of length
   * `size`: signed (towards the cursor), 0 in the middle, 1 at the edge. */
  private follow(position: number, size: number): number {
    const distance = Math.min(1, Math.abs(position - size / 2) / (size / 2))
    const speed = (Math.exp(FOLLOW_CURVE * distance) - 1) / (Math.exp(FOLLOW_CURVE) - 1)
    return position < size / 2 ? -speed : speed
  }

  private edgeStep = (now: number) => {
    this.edgeFrame = 0
    const pointer = this.pointer
    // Stops when the cursor leaves or the setting is off; a pointer move starts it again.
    if (!pointer || !this.navigation.edgeScroll) return
    const { width, height } = this.sigma.getDimensions()
    let dx = this.follow(pointer.x, width)
    let dy = this.follow(pointer.y, height)
    if (Math.abs(dx) < FOLLOW_MIN) dx = 0
    if (Math.abs(dy) < FOLLOW_MIN) dy = 0
    const seconds = Math.min(now - this.edgeLast, 50) / 1000
    this.edgeLast = now
    if (!dx && !dy) return
    // Resting on an item holds the view still, so it can be read or clicked; holding a button is
    // dragging the map by hand. Either way keep watching, and move on again once it's free.
    this.retarget()
    if (!this.dragging && !this.hovered) {
      const step = FOLLOW_SPEED * seconds
      const to = this.sigma.viewportToFramedGraph({ x: width / 2 + dx * width * step, y: height / 2 + dy * height * step })
      this.sigma.getCamera().setState(this.inside(to))
    }
    this.edgeFrame = requestAnimationFrame(this.edgeStep)
  }

  setNavigation(navigation: Navigation) {
    this.navigation = navigation
  }

  private onwheel = (event: WheelEvent) => {
    event.preventDefault()
    event.stopPropagation()
    const camera = this.sigma.getCamera()
    const rect = this.container.getBoundingClientRect()
    const cursor = { x: event.clientX - rect.left, y: event.clientY - rect.top }
    const lines = event.deltaMode === 1 ? 33 : 1
    const pixels = Math.max(-MAX_PIXELS, Math.min(MAX_PIXELS, event.deltaY * lines))
    // Scrolling up zooms in, like a map; the setting flips it.
    const direction = this.navigation.invertZoom ? 1 : -1
    const speed = event.ctrlKey ? PINCH_SPEED : WHEEL_SPEED
    if (!this.zoomFrame) {
      // A glide to a selected item may still be running; the scroll takes over from here.
      void camera.animate(camera.getState(), { duration: 1 })
      this.zoomAim = { ratio: camera.ratio, cursor }
    }
    const aim = this.zoomAim!
    aim.cursor = cursor
    aim.ratio = Math.min(MAX_RATIO, Math.max(MIN_RATIO, aim.ratio / Math.exp(pixels * speed * direction)))
    if (!this.zoomFrame) {
      this.zoomLast = performance.now()
      this.zoomFrame = requestAnimationFrame(this.zoomStep)
    }
  }

  /** Scrolling only sets where the zoom should end up; each frame the view covers part of the
   * way there. That stays smooth however fast or in what sizes the scroll events arrive. */
  private zoomStep = (now: number) => {
    this.zoomFrame = 0
    const aim = this.zoomAim
    if (!aim) return
    const camera = this.sigma.getCamera()
    const gap = Math.log(aim.ratio / camera.ratio)
    const done = Math.abs(gap) < 0.002
    const ease = 1 - Math.exp(-Math.min(now - this.zoomLast, 50) / ZOOM_EASE_MS)
    this.zoomLast = now
    const ratio = done ? aim.ratio : camera.ratio * Math.exp(gap * ease)
    // The cursor's distance from the view's centre grows in proportion to the ratio, so moving
    // the centre by that distance times the ratio's change keeps the point under the cursor.
    const under = this.sigma.viewportToFramedGraph(aim.cursor)
    const k = camera.ratio - ratio
    camera.setState({
      x: camera.x + ((under.x - camera.x) / camera.ratio) * k,
      y: camera.y + ((under.y - camera.y) / camera.ratio) * k,
      ratio,
    })
    this.retarget()
    if (done) this.zoomAim = null
    else this.zoomFrame = requestAnimationFrame(this.zoomStep)
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
    // Many items never placed (a library made or synced elsewhere): group them now.
    if (placed.length >= BURST_SIZE && this.graph.size > 0) void this.tidy()
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
      const { size, weight } = TIER_STYLE[edge.tier]
      this.graph.addEdge(edge.src, edge.dst, { size, weight, tier: edge.tier, color: this.edgeColor(edge.tier) })
    }
    this.resizeNodes()
  }

  /** Items with many sure links are drawn bigger; maybes don't count. */
  private resizeNodes() {
    this.graph.forEachNode((node) => {
      let links = 0
      this.graph.forEachEdge(node, (_, attrs) => (links += attrs.tier === 'maybe' ? 0 : 1))
      this.graph.setNodeAttribute(node, 'size', nodeSize(links))
    })
  }

  /** Items were deleted: they and their links leave the map; the rest stays where it is. */
  removeItems(ids: string[]) {
    // A running layout holds on to the nodes it is moving.
    this.layout?.kill()
    this.layout = undefined
    for (const id of ids) {
      if (this.graph.hasNode(id)) this.graph.dropNode(id)
      this.marked.delete(id)
      if (this.hovered === id) this.hovered = null
      if (this.selected === id) this.selected = null
    }
    this.resizeNodes()
    this.sigma.refresh()
  }

  /** Shows links down to `tier`: strong only, strong and related, or everything. */
  setLowestTier(tier: Tier) {
    this.lowestTier = tier
    this.sigma.refresh({ skipIndexation: true })
  }

  private edgeColor(tier: Tier): string {
    return tier === 'strong' ? this.colors.edgeStrong : tier === 'maybe' ? this.colors.edgeMaybe : this.colors.edge
  }

  /** Moves a newly linked item next to the items it's linked to (closer to stronger links). */
  settle(id: string) {
    if (!this.graph.hasNode(id)) return
    let x = 0
    let y = 0
    let total = 0
    this.graph.forEachEdge(id, (_edge, attrs, source, target) => {
      const other = this.graph.getNodeAttributes(source === id ? target : source)
      const weight = (attrs.weight as number) ?? 1
      x += other.x * weight
      y += other.y * weight
      total += weight
    })
    if (!total) return
    this.noteBurst()
    // Next to the neighbours rather than on top of them.
    const angle = Math.random() * 2 * Math.PI
    const offset = SPIRAL_SPACING * 0.8
    const target = { x: x / total + offset * Math.cos(angle), y: y / total + offset * Math.sin(angle) }
    this.moveNodes({ [id]: target }, SETTLE_MS, () => this.events.placed([{ id, ...target }]))
  }

  /** Lays the whole map out again by its links, so related items gather into groups. */
  tidy(): Promise<void> {
    clearTimeout(this.burstTimer)
    this.burst = 0
    this.layout?.kill()
    const graph = this.graph as unknown as Graph
    return new Promise((done) =>
      this.moveNodes(groupedStart(graph, SPIRAL_SPACING), GROUP_MS, () => {
        const layout = new FA2Layout(this.graph, { settings: forceSettings(graph), getEdgeWeight: 'weight' })
        this.layout = layout
        layout.start()
        setTimeout(() => {
          layout.kill()
          if (this.layout !== layout) return done() // Superseded by another tidy.
          this.layout = undefined
          for (const [id, position] of Object.entries(rescaled(graph, SPIRAL_SPACING))) {
            this.graph.mergeNodeAttributes(id, position)
          }
          this.events.placed(this.positions())
          this.sigma.getCamera().animatedReset({ duration: 300 })
          done()
        }, forceDuration(graph))
      }),
    )
  }

  /** A short force-layout pass from the current positions: newly linked items pull their groups
   * together and push unrelated ones apart, without regrouping or moving the camera. */
  private relax(): Promise<void> {
    if (this.layout || this.graph.size === 0) return Promise.resolve()
    const graph = this.graph as unknown as Graph
    const layout = new FA2Layout(this.graph, { settings: forceSettings(graph), getEdgeWeight: 'weight' })
    this.layout = layout
    layout.start()
    return new Promise((done) =>
      setTimeout(() => {
        layout.kill()
        if (this.layout !== layout) return done() // Superseded by a tidy or a delete.
        this.layout = undefined
        for (const [id, position] of Object.entries(rescaled(graph, SPIRAL_SPACING))) {
          this.graph.mergeNodeAttributes(id, position)
        }
        this.events.placed(this.positions())
        done()
      }, RELAX_MS),
    )
  }

  /** Animates nodes to new positions, then calls `done`. Animation needs animation frames, which
   * a hidden window doesn't get, so the positions are applied directly if it hasn't finished in
   * time. */
  private moveNodes(targets: Record<string, { x: number; y: number }>, duration: number, done: () => void) {
    let finished = false
    const finish = () => {
      if (finished) return
      finished = true
      clearTimeout(fallback)
      done()
    }
    const cancel = animateNodes(this.graph, targets, { duration }, finish)
    const fallback = setTimeout(() => {
      cancel()
      for (const [id, position] of Object.entries(targets)) {
        if (this.graph.hasNode(id)) this.graph.mergeNodeAttributes(id, position)
      }
      finish()
    }, duration + 500)
  }

  /** Counts settles in a burst and tidies once it has died down. */
  private noteBurst() {
    this.burst++
    clearTimeout(this.burstTimer)
    this.burstTimer = setTimeout(() => {
      // A big burst regroups from scratch; a few new items just let the map settle where it is.
      if (this.burst >= BURST_SIZE) void this.tidy()
      else if (this.burst > 0) void this.relax()
      this.burst = 0
    }, BURST_QUIET_MS)
  }

  select(id: string | null) {
    this.selected = id
    this.sigma.refresh({ skipIndexation: true })
    if (id) this.focus(id, true)
  }

  /** Items picked together; they are drawn like the selected one. */
  setMarked(ids: string[]) {
    this.marked = new Set(ids)
    this.sigma.refresh({ skipIndexation: true })
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

  /** Arrow keys: the view glides in that direction for as long as the key is held. */
  panKey(direction: PanDirection, down: boolean, fast = false) {
    if (!down) {
      this.panHeld.delete(direction)
      return
    }
    this.panHeld.add(direction)
    this.panFast = fast
    if (this.panFrame) return
    // A glide to a selected item may still be running; the keys take over from here.
    const camera = this.sigma.getCamera()
    void camera.animate(camera.getState(), { duration: 1 })
    this.measure()
    this.panLast = performance.now()
    this.panFrame = requestAnimationFrame(this.panStep)
  }

  /** Lets go of every arrow key (the window lost focus, so no key-up will come). */
  stopPan() {
    this.panHeld.clear()
  }

  private panStep = (now: number) => {
    this.panFrame = 0
    if (!this.panHeld.size) return
    const seconds = Math.min(now - this.panLast, 50) / 1000
    this.panLast = now
    const step = (this.panFast ? PAN_SPEED_FAST : PAN_SPEED) * seconds
    const { width, height } = this.sigma.getDimensions()
    const dx = (this.panHeld.has('right') ? 1 : 0) - (this.panHeld.has('left') ? 1 : 0)
    const dy = (this.panHeld.has('down') ? 1 : 0) - (this.panHeld.has('up') ? 1 : 0)
    const to = this.sigma.viewportToFramedGraph({ x: width / 2 + dx * width * step, y: height / 2 + dy * height * step })
    this.sigma.getCamera().setState(this.inside(to))
    this.retarget()
    this.panFrame = requestAnimationFrame(this.panStep)
  }

  /** Zooms in (`true`) or out about the middle of the view. */
  zoom(inwards: boolean) {
    const camera = this.sigma.getCamera()
    if (inwards) void camera.animatedZoom({ duration: 200 })
    else void camera.animatedUnzoom({ duration: 200 })
  }

  /** Shows the whole map. */
  showAll() {
    void this.sigma.getCamera().animatedReset({ duration: 300 })
  }

  /** Brings the selected item back to the middle of the view. */
  showSelected() {
    if (this.selected) this.focus(this.selected, true)
  }

  destroy() {
    cancelAnimationFrame(this.panFrame)
    cancelAnimationFrame(this.zoomFrame)
    cancelAnimationFrame(this.edgeFrame)
    this.container.removeEventListener('wheel', this.onwheel, { capture: true })
    this.container.removeEventListener('pointermove', this.onpointermove)
    this.container.removeEventListener('pointerleave', this.onpointerleave)
    window.removeEventListener('blur', this.onpointerleave)
    window.removeEventListener('pointerup', this.onpointerup)
    this.themeQuery.removeEventListener('change', this.onThemeChange)
    this.resizeObserver.disconnect()
    clearTimeout(this.burstTimer)
    this.layout?.kill()
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
    const focus = this.focused()
    const outsideSearch = this.matches !== null && !this.matches.has(node)
    const outsideFocus = focus !== null && node !== focus && !this.graph.areNeighbors(node, focus)
    if (outsideSearch || outsideFocus) {
      result.color = this.colors.dim
      result.label = ''
      result.zIndex = 0
    } else {
      result.zIndex = 1
    }
    if (node === this.selected || node === this.hovered || this.marked.has(node)) {
      result.highlighted = true
      result.forceLabel = true
      result.zIndex = 2
    }
    return result
  }

  private reduceEdge(edge: string, data: EdgeDisplayData): Partial<EdgeDisplayData> {
    const [a, b] = this.graph.extremities(edge)
    const focus = this.focused()
    if (focus && a !== focus && b !== focus) return { ...data, hidden: true }
    if (this.matches && !(this.matches.has(a) && this.matches.has(b))) return { ...data, hidden: true }
    const tier = this.graph.getEdgeAttribute(edge, 'tier') as Tier
    if (!focus && !tierShown(tier, this.lowestTier)) return { ...data, hidden: true }
    return data
  }

  /** The item whose neighbourhood stays bright: the one under the pointer, else the selected one. */
  private focused(): string | null {
    return this.hovered ?? this.selected
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

    // A soft shadow lifts the label off the map instead of an outline.
    context.save()
    context.shadowColor = 'rgb(0 0 0 / 0.28)'
    context.shadowBlur = 10
    context.shadowOffsetY = 2
    context.fillStyle = this.colors.surface
    context.beginPath()
    context.roundRect(x, y, width + data.size, height, 6)
    context.fill()
    context.restore()

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
    this.graph.forEachEdge((edge, attrs) => this.graph.setEdgeAttribute(edge, 'color', this.edgeColor(attrs.tier as Tier)))
  }
}

/** Better-connected items are drawn bigger. */
function nodeSize(degree: number): number {
  return 5 + 1.6 * Math.sqrt(degree)
}
