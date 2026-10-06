<script lang="ts">
  // The last two weeks of AI use as stacked bars, one colour per purpose, switchable between
  // requests and tokens. Hovering (or focusing) a day says exactly what it was.
  import type { UsageReport } from '../lib/types'

  let { days, names }: { days: UsageReport['days']; names: Record<string, string> } = $props()

  type Metric = 'calls' | 'tokens'
  let metric = $state<Metric>('calls')
  let hovered = $state<number | null>(null)

  /** A stable colour for each purpose (the theme's own colours, so dark mode works). */
  const COLOURS: Record<string, string> = {
    notes: 'var(--kind-text)',
    ask: 'var(--kind-code)',
    topics: 'var(--kind-url)',
    images: 'var(--kind-image)',
    check: 'var(--kind-file)',
    other: 'var(--kind-file)',
  }
  const colour = (purpose: string) => COLOURS[purpose] ?? 'var(--kind-file)'

  const value = (d: { calls: number; tokens: number }) => d[metric]
  const compact = (n: number) => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n))

  /** A round top for the scale: 1, 2, 5 times a power of ten, so the lines read as plain numbers. */
  function niceTop(max: number): number {
    if (max <= 4) return 4
    const power = 10 ** Math.floor(Math.log10(max))
    for (const step of [1, 2, 2.5, 5, 10]) if (step * power >= max) return step * power
    return 10 * power
  }

  const top = $derived(niceTop(Math.max(...days.map(value), 0)))
  const ticks = $derived([top, top / 2, 0])
  const total = $derived(days.reduce((n, d) => n + value(d), 0))
  const busy = $derived(days.filter((d) => value(d) > 0).length)
  /** The purposes that appear at all, busiest overall first, for the key. */
  const purposes = $derived(
    Object.entries(
      days.reduce<Record<string, number>>((all, d) => {
        for (const p of d.by) all[p.purpose] = (all[p.purpose] ?? 0) + p[metric]
        return all
      }, {}),
    )
      .filter(([, n]) => n > 0)
      .sort((a, b) => b[1] - a[1])
      .map(([name]) => name),
  )

  const date = (day: string) => new Date(`${day}T12:00:00`)
  const weekday = (day: string) => date(day).toLocaleDateString(undefined, { weekday: 'short' })
  const long = (day: string, i: number) => (i === days.length - 1 ? 'Today' : date(day).toLocaleDateString(undefined, { weekday: 'long', month: 'short', day: 'numeric' }))
  /** Under each bar: the weekday for the last week and the date number, with a month on the 1st. */
  const label = (day: string, i: number) => (i === days.length - 1 ? 'Today' : date(day).getDate() === 1 ? date(day).toLocaleDateString(undefined, { month: 'short' }) : String(date(day).getDate()))
  const unit = $derived(metric === 'calls' ? 'requests' : 'tokens')
</script>

<div class="chart">
  <div class="head">
    <p class="summary">
      {#if total > 0}
        <strong>{compact(total)}</strong>
        {unit} in {days.length} days{#if busy > 0}, <span class="muted">{compact(Math.round(total / busy))} on a day you used it</span>{/if}
      {:else}
        Nothing in the last {days.length} days
      {/if}
    </p>
    <div class="segmented" role="radiogroup" aria-label="Show">
      <button role="radio" aria-checked={metric === 'calls'} class:on={metric === 'calls'} onclick={() => (metric = 'calls')}>Requests</button>
      <button role="radio" aria-checked={metric === 'tokens'} class:on={metric === 'tokens'} onclick={() => (metric = 'tokens')}>Tokens</button>
    </div>
  </div>

  <div class="plot" role="group" aria-label={`${unit} per day over the last ${days.length} days`}>
    <div class="scale" aria-hidden="true">
      {#each ticks as t}<span>{t === 0 ? '' : compact(t)}</span>{/each}
    </div>
    <div class="area">
      <div class="lines" aria-hidden="true"><span></span><span></span><span></span></div>
      <div class="bars" style:grid-template-columns={`repeat(${days.length}, 1fr)`}>
        {#each days as d, i (d.day)}
          <button
            class="day"
            class:on={hovered === i}
            class:today={i === days.length - 1}
            onmouseenter={() => (hovered = i)}
            onmouseleave={() => (hovered = null)}
            onfocus={() => (hovered = i)}
            onblur={() => (hovered = null)}
            aria-label={`${long(d.day, i)}: ${value(d)} ${unit}`}
          >
            <span class="stack" style:height={`${(value(d) / top) * 100}%`}>
              {#each d.by as p (p.purpose)}
                {#if p[metric] > 0}<span class="part" style:flex-grow={p[metric]} style:background={colour(p.purpose)}></span>{/if}
              {/each}
            </span>
            {#if value(d) === 0}<span class="none"></span>{/if}
          </button>
        {/each}
      </div>
      {#if hovered !== null}
        {@const d = days[hovered]}
        <!-- Beside the bar, over the empty side of the plot, so the pane never clips it. -->
        <div class="tip" style:left={`${((hovered < days.length / 2 ? hovered + 1 : hovered) / days.length) * 100}%`} class:flip={hovered >= days.length / 2} role="status">
          <strong>{long(d.day, hovered)}</strong>
          {#if value(d) === 0}
            <span class="muted">Nothing</span>
          {:else}
            {#each d.by.filter((p) => p[metric] > 0) as p (p.purpose)}
              <span class="row"><i style:background={colour(p.purpose)}></i>{names[p.purpose] ?? p.purpose}<b>{compact(p[metric])}</b></span>
            {/each}
            {#if d.by.filter((p) => p[metric] > 0).length > 1}<span class="row total">Total<b>{compact(value(d))}</b></span>{/if}
          {/if}
        </div>
      {/if}
    </div>
    <div class="axis" style:grid-template-columns={`repeat(${days.length}, 1fr)`} aria-hidden="true">
      {#each days as d, i (d.day)}<span class:today={i === days.length - 1}>{label(d.day, i)}</span>{/each}
    </div>
  </div>

  {#if purposes.length}
    <ul class="key">
      {#each purposes as name (name)}<li><i style:background={colour(name)}></i>{names[name] ?? name}</li>{/each}
    </ul>
  {/if}
</div>

<style>
  .chart {
    padding: 12px 14px 14px;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 14px;
  }

  .summary {
    margin: 0;
    font-size: 13px;
  }

  .summary strong {
    font-size: 18px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .muted {
    color: var(--text-muted);
  }

  .segmented {
    display: inline-flex;
    flex: none;
    padding: 2px;
    border-radius: var(--radius-sm);
    background: var(--fill);
  }

  .segmented button {
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .segmented button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-sm);
  }

  .plot {
    display: grid;
    grid-template-columns: 34px 1fr;
    grid-template-rows: 150px auto;
    column-gap: 6px;
  }

  .scale {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    margin-top: -7px;
    color: var(--text-muted);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    line-height: 14px;
  }

  .scale span:last-child {
    height: 14px;
  }

  .area {
    position: relative;
  }

  .lines {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    pointer-events: none;
  }

  .lines span {
    border-top: 1px dashed var(--line);
  }

  .lines span:last-child {
    border-top-style: solid;
  }

  .bars {
    position: absolute;
    inset: 0;
    display: grid;
    gap: 4px;
  }

  .day {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    padding: 0;
    border: none;
    border-radius: 4px 4px 0 0;
    background: none;
    outline-offset: 1px;
  }

  .day.on {
    background: var(--fill);
  }

  .stack {
    display: flex;
    flex-direction: column-reverse;
    width: 100%;
    min-height: 0;
    overflow: hidden;
    border-radius: 3px 3px 0 0;
    opacity: 0.85;
    transition: height 300ms;
  }

  .day.on .stack,
  .day.today .stack {
    opacity: 1;
  }

  .part {
    flex-basis: 0;
    min-height: 2px;
  }

  .part + .part {
    border-bottom: 1px solid var(--surface);
  }

  .none {
    position: absolute;
    left: 20%;
    right: 20%;
    bottom: 0;
    height: 2px;
    border-radius: 1px;
    background: var(--fill-hover);
  }

  .axis {
    grid-column: 2;
    display: grid;
    gap: 4px;
    margin-top: 6px;
    color: var(--text-muted);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .axis .today {
    color: var(--text);
    font-weight: 600;
  }

  .tip {
    position: absolute;
    top: 0;
    z-index: 2;
    margin: 0 6px;
    white-space: nowrap;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 150px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--popover);
    box-shadow: var(--shadow);
    font-size: 12px;
    pointer-events: none;
  }

  .tip.flip {
    transform: translateX(-100%);
  }

  .tip strong {
    font-weight: 600;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .row b {
    margin-left: auto;
    padding-left: 12px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .row.total {
    margin-top: 2px;
    padding-top: 4px;
    border-top: 1px solid var(--line);
    color: var(--text-muted);
  }

  .row i,
  .key i {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .key {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin: 12px 0 0 40px;
    padding: 0;
    list-style: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .key li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
