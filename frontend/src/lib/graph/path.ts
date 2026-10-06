// The Ask agent's path on the map: a canvas laid over Sigma's. While a question runs, the notes it
// has seen get a faint ring, the ones it has read a stronger one, a thin line joins them in the
// order they were read, and the latest step pulses. Once it has answered, the line fades and the
// notes the answer cites keep a ring.
//
// It only draws while it is switched on (the Ask panel is showing) and the window is in front,
// and it steps aside whenever the person is working the map themselves (dragging, zooming,
// panning, or resting on a note) until they have been still for a moment.

import type Sigma from 'sigma'

/** How long the map must be left alone before the path comes back. */
const QUIET_MS = 1500
const PULSE_MS = 1100
/** After an answer, how long the line and the faint rings take to fade. */
const FADE_MS = 3000

export interface PathStep {
  tool: string
  items: string[]
}

export interface PathState {
  /** The Ask panel is showing. */
  on: boolean
  running: boolean
  steps: PathStep[]
  cited: string[]
  /** Bring the latest note into view when it is out of sight (never while the map is in use). */
  follow: boolean
}

const EMPTY: PathState = { on: false, running: false, steps: [], cited: [], follow: false }

export class PathLayer {
  private canvas: HTMLCanvasElement
  private state: PathState = EMPTY
  private accent: string
  private quietUntil = 0
  private quietTimer: ReturnType<typeof setTimeout> | undefined
  private pulseStart = 0
  private pulseStep = 0
  private finishedAt = 0
  private frame = 0
  private render = () => this.draw()

  constructor(
    container: HTMLElement,
    private sigma: Sigma,
    accent: string,
  ) {
    this.accent = accent
    this.canvas = document.createElement('canvas')
    this.canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;pointer-events:none'
    this.canvas.setAttribute('aria-hidden', 'true')
    container.appendChild(this.canvas)
    sigma.on('afterRender', this.render)
    document.addEventListener('visibilitychange', this.render)
  }

  setColor(accent: string) {
    this.accent = accent
    this.draw()
  }

  set(next: PathState) {
    const before = this.state
    this.state = next
    if (next.steps.length > before.steps.length && next.running) {
      this.pulseStart = performance.now()
      this.pulseStep = next.steps.length
    }
    if (before.running && !next.running) this.finishedAt = performance.now()
    if (next.running) this.finishedAt = 0
    this.draw()
  }

  /** Whether the person is using the map right now. */
  interacting(): boolean {
    return performance.now() < this.quietUntil
  }

  /** The person is using the map: hide the path, and bring it back once they stop. */
  touch() {
    if (!this.state.on) return
    const wasQuiet = performance.now() < this.quietUntil
    this.quietUntil = performance.now() + QUIET_MS
    clearTimeout(this.quietTimer)
    this.quietTimer = setTimeout(() => this.draw(), QUIET_MS + 20)
    if (!wasQuiet) this.draw()
  }

  destroy() {
    cancelAnimationFrame(this.frame)
    clearTimeout(this.quietTimer)
    this.sigma.off('afterRender', this.render)
    document.removeEventListener('visibilitychange', this.render)
    this.canvas.remove()
  }

  private clear() {
    const { width, height } = this.sigma.getDimensions()
    const ratio = window.devicePixelRatio || 1
    const w = Math.round(width * ratio)
    const h = Math.round(height * ratio)
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w
      this.canvas.height = h
    }
    const ctx = this.canvas.getContext('2d')!
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
    ctx.clearRect(0, 0, width, height)
    return ctx
  }

  private at(id: string): { x: number; y: number; r: number } | null {
    const display = this.sigma.getNodeDisplayData(id)
    if (!display || display.hidden) return null
    const { x, y } = this.sigma.framedGraphToViewport(display)
    return { x, y, r: this.sigma.scaleSize(display.size) }
  }

  private draw() {
    cancelAnimationFrame(this.frame)
    const ctx = this.clear()
    const now = performance.now()
    const { on, running, steps, cited } = this.state
    if (!on || document.hidden || now < this.quietUntil) return

    // The line and the faint rings fade away once the answer is in.
    const fade = running ? 1 : Math.max(0, 1 - (now - this.finishedAt) / FADE_MS)
    const read: string[] = []
    const seen = new Set<string>()
    for (const step of steps) {
      for (const id of step.items) seen.add(id)
      if (step.tool === 'read' && read.at(-1) !== step.items[0]) read.push(step.items[0])
    }
    let animating = !running && fade > 0 && steps.length > 0

    ctx.lineCap = 'round'
    ctx.strokeStyle = this.accent
    ctx.fillStyle = this.accent

    if (fade > 0) {
      // Everything it was shown.
      for (const id of seen) {
        const p = this.at(id)
        if (!p) continue
        ctx.globalAlpha = 0.35 * fade
        ctx.lineWidth = 1.5
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.r + 3, 0, Math.PI * 2)
        ctx.stroke()
      }
      // The line through what it read, newest part strongest.
      const points = read.map((id) => this.at(id)).filter((p) => p !== null)
      for (let i = 1; i < points.length; i++) {
        const [a, b] = [points[i - 1], points[i]]
        const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }
        const bend = 0.15
        ctx.globalAlpha = (0.25 + 0.5 * (i / (points.length - 1 || 1))) * fade
        ctx.lineWidth = 2
        ctx.beginPath()
        ctx.moveTo(a.x, a.y)
        ctx.quadraticCurveTo(mid.x - (b.y - a.y) * bend, mid.y + (b.x - a.x) * bend, b.x, b.y)
        ctx.stroke()
      }
      for (const p of points) {
        ctx.globalAlpha = 0.85 * fade
        ctx.lineWidth = 2.5
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.r + 4, 0, Math.PI * 2)
        ctx.stroke()
      }
    }

    // The latest step: a ring that spreads and fades around what it just looked at.
    const progress = (now - this.pulseStart) / PULSE_MS
    if (running && this.pulseStep && progress < 1) {
      animating = true
      for (const id of steps[this.pulseStep - 1]?.items ?? []) {
        const p = this.at(id)
        if (!p) continue
        ctx.globalAlpha = 0.7 * (1 - progress)
        ctx.lineWidth = 2
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.r + 6 + 26 * progress, 0, Math.PI * 2)
        ctx.stroke()
      }
    }

    // What the answer rests on keeps a ring for as long as the panel is showing.
    if (!running) {
      for (const id of cited) {
        const p = this.at(id)
        if (!p) continue
        ctx.globalAlpha = 0.95
        ctx.lineWidth = 3
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.r + 5, 0, Math.PI * 2)
        ctx.stroke()
        ctx.globalAlpha = 0.18
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.r + 11, 0, Math.PI * 2)
        ctx.fill()
      }
    }
    ctx.globalAlpha = 1
    if (animating) this.frame = requestAnimationFrame(this.render)
  }
}
