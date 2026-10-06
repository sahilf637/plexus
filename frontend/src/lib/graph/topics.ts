// Topics on the map: a soft shaded region behind each group of items and its name on top,
// changing with the zoom. Zoomed out you see the big topics; as you zoom in a topic gives way to
// its sub-topics, and finally to the items themselves. Drawn on two canvases laid over the map
// (one under Sigma's, for the regions; one over it, for the names), redrawn after every render.

import type Graph from 'graphology'
import type Sigma from 'sigma'
import type { Topic, TopicsData } from '../types'

/** Smallest on-screen radius at which a topic is named, in pixels. */
const MIN_RADIUS = 34
/** A topic's name gives way to its sub-topics' once those are this big: from `[0]` to `[1]` times
 * MIN_RADIUS, or that share of the view's shorter side if more (so on a big screen too, the top
 * topics hold until you have zoomed in properly). */
const CHILD_TAKEOVER = [1.3, 2.0]
const CHILD_VIEW_SHARE = [0.16, 0.26]
/** Names fade out when the topic is this big on screen (we are inside it; its items show). This
 * starts only once the take-over from a parent's name is complete (see `draw`), so a name that has
 * just taken over is not already fading. */
const LEAF_FADE = [90, 170]
/** Zoomed right out (the whole map in view), top-level names stay at least this visible: they are
 * the map's legend. */
const OVERVIEW_FLOOR = 0.85
/** The camera ratio between which that floor comes in (1 shows the whole map). */
const OVERVIEW_RATIO = [0.55, 0.85]
/** ...or, for a topic with sub-topics, this share of the view's shorter side. */
const INSIDE_FADE = [0.55, 0.95]
/** Spread of a topic's items to region radius: how far past the typical distance it reaches. */
const REACH = 1.5
const FONT = ['600 15px', '600 13.5px', '600 12.5px']
const PAD = 6

const smooth = (value: number, from: number, to: number) => {
  const t = Math.min(1, Math.max(0, (value - from) / (to - from)))
  return t * t * (3 - 2 * t)
}

interface Shape {
  /** Centre and radius in graph coordinates. */
  x: number
  y: number
  radius: number
}

interface Placed {
  id: string
  left: number
  top: number
  right: number
  bottom: number
}

export interface TopicColors {
  text: string
  surface: string
  accent: string
  dark: boolean
}

export class TopicLayer {
  private under: HTMLCanvasElement
  private over: HTMLCanvasElement
  private topics = new Map<string, Topic>()
  private children = new Map<string, string[]>()
  /** Items in each topic and its sub-topics. */
  private members = new Map<string, string[]>()
  private hue = new Map<string, number>()
  private shapes = new Map<string, Shape>()
  private signature = ''
  private placed: Placed[] = []
  private selected: string | null = null
  private colors: TopicColors
  private lastKey = ''
  private labelsHidden = false
  private render = () => this.draw()

  constructor(
    container: HTMLElement,
    private sigma: Sigma,
    private graph: Graph,
    colors: TopicColors,
    /** Told when the map's own item labels should hide (topic names are doing the talking). */
    private itemLabels: (on: boolean) => void,
  ) {
    this.colors = colors
    const make = () => {
      const canvas = document.createElement('canvas')
      canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;pointer-events:none'
      canvas.setAttribute('aria-hidden', 'true')
      return canvas
    }
    this.under = make()
    this.over = make()
    // Under Sigma's own layers, and over them.
    container.insertBefore(this.under, container.firstChild)
    container.appendChild(this.over)
    sigma.on('afterRender', this.render)
  }

  setColors(colors: TopicColors) {
    this.colors = colors
    this.lastKey = ''
    this.draw()
  }

  /** The topic tree changed, or an item moved between topics. */
  setTopics(data: TopicsData | null) {
    this.topics.clear()
    this.children.clear()
    this.members.clear()
    this.hue.clear()
    this.shapes.clear()
    this.signature = ''
    this.lastKey = ''
    if (!data) {
      this.draw()
      return
    }
    for (const topic of data.topics) this.topics.set(topic.id, topic)
    for (const topic of data.topics) {
      if (topic.parentId && this.topics.has(topic.parentId)) {
        const list = this.children.get(topic.parentId) ?? []
        list.push(topic.id)
        this.children.set(topic.parentId, list)
      }
    }
    for (const [item, leaf] of Object.entries(data.itemTopic)) {
      for (let at: string | null | undefined = leaf; at; at = this.topics.get(at)?.parentId) {
        const list = this.members.get(at) ?? []
        list.push(item)
        this.members.set(at, list)
      }
    }
    // Each top-level topic gets its own hue, spread around the colour wheel; its sub-topics share it.
    let n = 0
    for (const topic of data.topics.filter((t) => !t.parentId)) {
      const hue = (n++ * 137.5) % 360
      const paint = (id: string) => {
        this.hue.set(id, hue)
        this.children.get(id)?.forEach(paint)
      }
      paint(topic.id)
    }
    this.draw()
  }

  /** The topic shown as picked, if any. */
  setSelected(id: string | null) {
    this.selected = id
    this.lastKey = ''
    this.draw()
  }

  /** The topic whose name is under this point (in pixels from the map's top-left corner). */
  hit(x: number, y: number): string | null {
    for (let i = this.placed.length - 1; i >= 0; i--) {
      const p = this.placed[i]
      if (x >= p.left && x <= p.right && y >= p.top && y <= p.bottom) return p.id
    }
    return null
  }

  destroy() {
    this.sigma.off('afterRender', this.render)
    this.under.remove()
    this.over.remove()
  }

  /** Where each topic sits now: the middle of its items and how far they spread. Only worked out
   * again when something moved. */
  private measure() {
    let sum = this.graph.order
    this.graph.forEachNode((_, a) => (sum += a.x * 3 + a.y))
    const signature = `${this.graph.order}:${sum.toFixed(3)}`
    if (signature === this.signature && this.shapes.size) return
    this.signature = signature
    this.shapes.clear()
    for (const [id, items] of this.members) {
      let x = 0
      let y = 0
      let count = 0
      for (const item of items) {
        if (!this.graph.hasNode(item)) continue
        x += this.graph.getNodeAttribute(item, 'x')
        y += this.graph.getNodeAttribute(item, 'y')
        count++
      }
      if (!count) continue
      x /= count
      y /= count
      let spread = 0
      for (const item of items) {
        if (!this.graph.hasNode(item)) continue
        const dx = this.graph.getNodeAttribute(item, 'x') - x
        const dy = this.graph.getNodeAttribute(item, 'y') - y
        spread += dx * dx + dy * dy
      }
      this.shapes.set(id, { x, y, radius: REACH * Math.sqrt(spread / count) })
    }
  }

  private fit(canvas: HTMLCanvasElement, width: number, height: number) {
    const ratio = window.devicePixelRatio || 1
    const w = Math.round(width * ratio)
    const h = Math.round(height * ratio)
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w
      canvas.height = h
    }
    const ctx = canvas.getContext('2d')!
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
    ctx.clearRect(0, 0, width, height)
    return ctx
  }

  private draw() {
    const { width, height } = this.sigma.getDimensions()
    const camera = this.sigma.getCamera()
    this.measure()
    const key = [width, height, camera.x.toFixed(5), camera.y.toFixed(5), camera.ratio.toFixed(5), this.signature, this.selected, this.colors.dark].join('|')
    if (key === this.lastKey) return
    this.lastKey = key

    const regions = this.fit(this.under, width, height)
    const names = this.fit(this.over, width, height)
    this.placed = []
    if (!this.topics.size || width === 0) {
      if (this.labelsHidden) {
        this.labelsHidden = false
        this.itemLabels(true)
      }
      return
    }

    // Pixels per unit of graph space, here.
    const origin = this.sigma.graphToViewport({ x: 0, y: 0 })
    const unit = Math.hypot(this.sigma.graphToViewport({ x: 1, y: 0 }).x - origin.x, this.sigma.graphToViewport({ x: 1, y: 0 }).y - origin.y)
    const radiusOf = (id: string) => (this.shapes.get(id)?.radius ?? 0) * unit

    // How visible each topic's name is at this zoom. A topic's name gives way to its sub-topics'
    // as those grow, and a sub-topic's name only appears as it takes over from its parent's.
    const view = Math.min(width, height)
    const takeovers = new Map<string, number>()
    const takeoverOf = (id: string): number => {
      const known = takeovers.get(id)
      if (known !== undefined) return known
      const kids = this.children.get(id) ?? []
      const from = Math.max(MIN_RADIUS * CHILD_TAKEOVER[0], view * CHILD_VIEW_SHARE[0])
      const to = Math.max(MIN_RADIUS * CHILD_TAKEOVER[1], view * CHILD_VIEW_SHARE[1])
      const value = kids.length ? Math.max(...kids.map((k) => smooth(radiusOf(k), from, to))) : 0
      takeovers.set(id, value)
      return value
    }
    const appearing = (id: string): number => {
      const parent = this.topics.get(id)?.parentId
      return parent && this.topics.has(parent) ? takeoverOf(parent) * appearing(parent) : 1
    }
    // A name that has just taken over from its parent's must not already be fading as a leaf.
    const takeoverEnd = Math.max(MIN_RADIUS * CHILD_TAKEOVER[1], view * CHILD_VIEW_SHARE[1])
    const leafFrom = Math.max(LEAF_FADE[0], takeoverEnd + 10)
    const leafTo = Math.max(LEAF_FADE[1], leafFrom + 80)
    // 1 when the whole map is in view (ratio 1 or more), 0 once zoomed in.
    const overview = smooth(camera.ratio, OVERVIEW_RATIO[0], OVERVIEW_RATIO[1])
    const alpha = new Map<string, number>()
    /** The same without the picked topic's boost: what the zoom alone says. */
    const natural = new Map<string, number>()
    for (const [id, topic] of this.topics) {
      const r = radiusOf(id)
      const kids = this.children.get(id) ?? []
      let a = smooth(r, MIN_RADIUS * 0.8, MIN_RADIUS * 1.2) * appearing(id)
      if (kids.length) {
        a *= 1 - takeoverOf(id)
        a *= 1 - smooth(r, view * INSIDE_FADE[0], view * INSIDE_FADE[1])
      } else {
        a *= 1 - smooth(r, leafFrom, leafTo)
      }
      if (a > 0.02 && topic.count > 0) natural.set(id, a)
      // At overview the top-level names are always there, as the map's legend.
      if (!topic.parentId && topic.count > 0 && this.shapes.has(id)) a = Math.max(a, overview * OVERVIEW_FLOOR)
      // The picked topic keeps its name whatever the zoom.
      if (id === this.selected) a = Math.max(a, 0.9)
      if (a > 0.02 && topic.count > 0) alpha.set(id, a)
    }

    // Regions first, biggest first so the small ones sit on top.
    const visible = [...alpha.keys()].sort((a, b) => radiusOf(b) - radiusOf(a))
    for (const id of visible) {
      const shape = this.shapes.get(id)
      if (!shape) continue
      const at = this.sigma.graphToViewport({ x: shape.x, y: shape.y })
      const r = radiusOf(id) * 1.2
      if (at.x + r < 0 || at.y + r < 0 || at.x - r > width || at.y - r > height) continue
      const level = this.topics.get(id)!.level
      const hue = this.hue.get(id) ?? 0
      const strength = (this.colors.dark ? 0.2 : 0.17) * alpha.get(id)! * (id === this.selected ? 1.4 : 1)
      const light = this.colors.dark ? 62 - level * 4 : 52 - level * 3
      const gradient = regions.createRadialGradient(at.x, at.y, 0, at.x, at.y, r)
      gradient.addColorStop(0, `hsla(${hue}, 55%, ${light}%, ${strength})`)
      gradient.addColorStop(0.65, `hsla(${hue}, 55%, ${light}%, ${strength * 0.55})`)
      gradient.addColorStop(1, `hsla(${hue}, 55%, ${light}%, 0)`)
      regions.fillStyle = gradient
      regions.beginPath()
      regions.arc(at.x, at.y, r, 0, Math.PI * 2)
      regions.fill()
    }

    // Names, the biggest topics first, skipping any that would land on one already placed. How
    // clearly names are showing (those really drawn, by zoom alone) decides about item labels.
    let shown = 0
    names.textBaseline = 'middle'
    names.textAlign = 'center'
    const byCount = visible.slice().sort((a, b) => (this.topics.get(b)!.count - this.topics.get(a)!.count) || a.localeCompare(b))
    for (const id of byCount) {
      const topic = this.topics.get(id)!
      const shape = this.shapes.get(id)
      if (!shape) continue
      const at = this.sigma.graphToViewport({ x: shape.x, y: shape.y })
      names.font = `${FONT[Math.min(topic.level, FONT.length - 1)]} system-ui, sans-serif`
      const label = topic.label
      const textWidth = names.measureText(label).width
      const half = textWidth / 2 + PAD
      const boxAt = (x: number, y: number): Placed => ({ id, left: x - half, right: x + half, top: y - 11 - PAD / 2, bottom: y + 11 + PAD / 2 })
      const taken = (r: Placed) => this.placed.some((p) => p.left < r.right && p.right > r.left && p.top < r.bottom && p.bottom > r.top)
      // If another name is in the way, this one slides off it by just the overlap, along the
      // axis where that is least. The slide grows and shrinks smoothly with the zoom, so names
      // ease apart instead of jumping. It stays within the topic's own region, else it hides.
      let x = at.x
      let y = at.y
      if (id !== this.selected) {
        const reach = radiusOf(id) * 0.8
        for (let pass = 0; pass < 4; pass++) {
          const here = boxAt(x, y)
          const other = this.placed.find((p) => p.left < here.right && p.right > here.left && p.top < here.bottom && p.bottom > here.top)
          if (!other) break
          const right = other.right - here.left
          const left = here.right - other.left
          const down = other.bottom - here.top
          const up = here.bottom - other.top
          const sideways = right < left ? right : -left
          const vertical = down < up ? down : -up
          if (Math.abs(vertical) <= Math.abs(sideways)) y += vertical
          else x += sideways
        }
        const blocked = this.placed.some((p) => {
          const r = boxAt(x, y)
          return p.left < r.right && p.right > r.left && p.top < r.bottom && p.bottom > r.top
        })
        if (blocked || Math.hypot(x - at.x, y - at.y) > reach) continue
      }
      const rect = boxAt(x, y)
      if (rect.right < 0 || rect.bottom < 0 || rect.left > width || rect.top > height) continue
      this.placed.push(rect)
      shown = Math.max(shown, natural.get(id) ?? 0)
      const a = alpha.get(id)!
      names.globalAlpha = a
      names.lineWidth = 4
      names.lineJoin = 'round'
      names.strokeStyle = this.colors.surface
      names.strokeText(label, x, y)
      names.fillStyle = id === this.selected ? this.colors.accent : this.colors.text
      names.fillText(label, x, y)
      names.globalAlpha = 1
    }

    // While topic names are clearly showing, the items' own labels would only crowd them; once
    // the names fade (zoomed in past them) the items speak for themselves.
    const hide = this.labelsHidden ? shown > 0.4 : shown > 0.55
    if (hide !== this.labelsHidden) {
      this.labelsHidden = hide
      this.itemLabels(!hide)
    }
  }
}
