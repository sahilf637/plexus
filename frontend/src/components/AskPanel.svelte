<script lang="ts">
  // Ask: a question about the library, answered from the notes, with the notes it rests on.
  // Docked at the side like the item and topic panels.
  import { ask, newAsk, removeAsk, reopen, resumeAsk, running, startAsk, stopAsk, trail } from '../lib/ask.svelte'
  import type { Ask, Item } from '../lib/types'
  import Icon from './Icon.svelte'

  let {
    items,
    onselect,
    onclose,
    onsettings,
  }: {
    items: Map<string, Item>
    /** A cited note (or a search result) was clicked. */
    onselect: (id: string) => void
    onclose: () => void
    /** Opens the AI part of Settings. */
    onsettings: () => void
  } = $props()

  let draft = $state('')
  let box = $state<HTMLTextAreaElement>()
  let scroller = $state<HTMLDivElement>()

  const current = $derived(ask.current)
  const last = $derived(ask.steps.at(-1))
  const looked = $derived(trail().length)

  /** The answer cut into plain text and citation marks, so each mark can be a button. */
  const parts = $derived.by(() => {
    const text = current?.answer ?? ''
    const out: ({ text: string } | { cite: number[] })[] = []
    let from = 0
    for (const match of text.matchAll(/\[(\d{1,2}(?:\s*,\s*\d{1,2})*)\]/g)) {
      if (match.index > from) out.push({ text: text.slice(from, match.index) })
      out.push({ cite: match[1].split(',').map((n) => Number(n.trim())) })
      from = match.index + match[0].length
    }
    if (from < text.length) out.push({ text: text.slice(from) })
    return out
  })

  const titleOf = (id: string) => items.get(id)?.title ?? 'a deleted note'
  const sourceOf = (n: number) => current?.sources.find((s) => s.n === n)

  function submit() {
    const question = draft
    if (!question.trim()) return
    draft = ''
    resize()
    void startAsk(question)
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault()
      submit()
    }
  }

  function resize() {
    if (!box) return
    box.style.height = 'auto'
    box.style.height = `${Math.min(box.scrollHeight, 140)}px`
  }

  // The newest line is in view as the answer arrives.
  $effect(() => {
    void ask.steps.length
    void current?.state
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' })
  })

  function when(past: Ask): string {
    const date = new Date(past.createdAt)
    return Number.isNaN(date.getTime()) ? '' : date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }

  const stateNote = $derived.by(() => {
    if (!current) return ''
    if (current.state === 'partial') return 'I ran out of steps, so this may be incomplete.'
    if (current.state === 'stopped') return 'Stopped early; this is what was found so far.'
    if (current.unsupported) return "I couldn't tie this to your notes, so treat it with care."
    return ''
  })
</script>

<aside>
  <header>
    <span class="chip"><Icon name="sparkles" size={13} />Ask</span>
    <span class="gap"></span>
    <span class="tools">
      <button
        class="icon-btn"
        class:on={ask.follow}
        aria-pressed={ask.follow}
        onclick={() => (ask.follow = !ask.follow)}
        title={ask.follow ? 'The map follows where I look (click to turn off)' : 'Let the map follow where I look'}
        aria-label="Follow on the map"
      >
        <Icon name="target" />
      </button>
      <button class="icon-btn" onclick={newAsk} title="New question" aria-label="New question"><Icon name="plus" /></button>
      <button
        class="icon-btn"
        class:on={ask.historyOpen}
        onclick={() => (ask.historyOpen = !ask.historyOpen)}
        title="Past questions"
        aria-label="Past questions"
      >
        <Icon name="history" />
      </button>
      <button class="icon-btn" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
    </span>
  </header>

  <div class="scroll" bind:this={scroller}>
    {#if ask.historyOpen}
      {#if ask.history.length}
        <ul class="history">
          {#each ask.history as past (past.id)}
            <li>
              <button onclick={() => reopen(past)}>
                <span class="q">{past.question}</span>
                <span class="when">{when(past)}</span>
              </button>
              <button class="icon-btn" onclick={() => removeAsk(past.id)} title="Delete" aria-label="Delete"><Icon name="trash" size={14} /></button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No questions yet.</p>
      {/if}
    {:else if current}
      <p class="question">{current.question}</p>
      {#if current.scope && ask.scope}<p class="scope">About {ask.scope.label}</p>{/if}

      {#if running()}
        <p class="progress">
          <span class="spin"></span>
          {last ? last.label[0].toUpperCase() + last.label.slice(1) : 'Starting'}…
          {#if looked > 1}<span class="muted">{looked} notes so far</span>{/if}
        </p>
      {:else if current.answer}
        <p class="answer">
          {#each parts as part}{#if 'text' in part}{part.text}{:else}{#each part.cite as n}<button
                  class="cite"
                  title={sourceOf(n) ? `“${sourceOf(n)!.quote}” — ${titleOf(sourceOf(n)!.itemId)}` : ''}
                  disabled={!sourceOf(n)}
                  onclick={() => sourceOf(n) && onselect(sourceOf(n)!.itemId)}>{n}</button
                >{/each}{/if}{/each}
        </p>
        {#if stateNote}<p class="note">{stateNote}</p>{/if}
        {#if current.sources.length}
          <h3>Sources</h3>
          <ol class="sources">
            {#each current.sources as source (source.n)}
              <li>
                <button onclick={() => onselect(source.itemId)}>
                  <span class="num">{source.n}</span>
                  <span class="what">
                    <span class="title">{titleOf(source.itemId)}</span>
                    <span class="quote">“{source.quote}”</span>
                  </span>
                </button>
              </li>
            {/each}
          </ol>
        {/if}
      {/if}

      {#if ask.error}
        <p class="error" role="alert">{ask.error}</p>
        {#if ask.canResume}<button class="btn" onclick={resumeAsk}><Icon name="refresh" size={14} />Continue</button>{/if}
      {/if}
    {:else if ask.fallback}
      <p class="question">{ask.fallback.question}</p>
      <p class="note">
        Connect an AI to get written answers.
        <button class="link" onclick={onsettings}>Open settings</button>
      </p>
      {#if ask.fallback.ids.length}
        <h3>Notes that match</h3>
        <ul class="matches">
          {#each ask.fallback.ids as id (id)}
            <li><button onclick={() => onselect(id)}>{titleOf(id)}</button></li>
          {/each}
        </ul>
      {:else}
        <p class="empty">Nothing in your notes matches.</p>
      {/if}
    {:else}
      <div class="intro">
        <p class="lead">Ask a question about your notes.</p>
        <p class="muted">Answers come only from what you've saved, with the notes they rest on.</p>
        {#if ask.scope}
          <p class="scope">
            About {ask.scope.label}
            <button class="link" onclick={() => (ask.scope = null)}>Ask about everything instead</button>
          </p>
        {/if}
        {#if ask.error}<p class="error" role="alert">{ask.error}</p>{/if}
      </div>
    {/if}
  </div>

  <form
    class="compose"
    onsubmit={(event) => {
      event.preventDefault()
      submit()
    }}
  >
    <textarea
      bind:this={box}
      bind:value={draft}
      {onkeydown}
      oninput={resize}
      rows="1"
      placeholder={ask.scope ? `Ask about ${ask.scope.label}` : 'Ask about your notes'}
      aria-label="Question"
    ></textarea>
    {#if running()}
      <button type="button" class="icon-btn" onclick={stopAsk} title="Stop" aria-label="Stop"><Icon name="stop" /></button>
    {:else}
      <button type="submit" class="icon-btn send" disabled={!draft.trim()} title="Ask (Enter)" aria-label="Ask"><Icon name="send" /></button>
    {/if}
  </form>
</aside>

<style>
  aside {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(400px, 100%);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
    box-shadow: -12px 0 28px -16px rgb(0 0 0 / 0.45);
    animation: slide-in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }

  @keyframes slide-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    aside {
      animation: none;
    }
  }

  header {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px 8px 16px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    padding: 3px 9px 3px 7px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .gap {
    flex: 1;
  }

  .tools {
    display: flex;
    gap: 2px;
    margin-right: -6px;
  }

  .icon-btn.on {
    background: var(--fill);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 16px 16px;
    overflow-y: auto;
  }

  .question {
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 600;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .scope,
  .muted,
  .note,
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--text-muted);
  }

  .spin {
    flex: none;
    width: 10px;
    height: 10px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation-duration: 3s;
    }
  }

  .answer {
    margin: 0;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .cite {
    display: inline-block;
    min-width: 16px;
    height: 16px;
    margin: 0 1px;
    padding: 0 4px;
    border: none;
    border-radius: 8px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    line-height: 16px;
    vertical-align: 1px;
  }

  .cite:hover:not(:disabled) {
    background: var(--accent);
    color: white;
  }

  .cite:disabled {
    background: var(--fill);
    color: var(--text-muted);
  }

  h3 {
    margin: 10px 0 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  ol,
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .sources li button,
  .matches li button,
  .history li > button:first-child {
    width: calc(100% + 16px);
    margin: 0 -8px;
    padding: 6px 8px;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    border: none;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  .sources li button:hover,
  .matches li button:hover,
  .history li > button:first-child:hover {
    background: var(--fill);
  }

  .num {
    flex: none;
    width: 18px;
    height: 18px;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    line-height: 18px;
    text-align: center;
  }

  .what {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quote {
    color: var(--text-muted);
    font-size: 12px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .history li {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .history li > button:first-child {
    flex: 1;
    justify-content: space-between;
    align-items: baseline;
  }

  .q {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .when {
    flex: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .intro {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 24px;
  }

  .lead {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .error {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: 13px;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-size: inherit;
    text-decoration: underline;
  }

  .compose {
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 10px 12px 12px 16px;
    border-top: 1px solid var(--line);
  }

  textarea {
    flex: 1;
    min-width: 0;
    max-height: 140px;
    padding: 7px 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--text);
    font: inherit;
    line-height: 1.4;
    resize: none;
    outline: none;
  }

  textarea:focus {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .send:not(:disabled) {
    color: var(--accent);
  }
</style>
