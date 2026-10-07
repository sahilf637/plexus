<script lang="ts">
  // Ask: a question about the library, answered from the notes, with the notes it rests on.
  // Docked at the side like the item and topic panels.
  import { toast } from '../lib/toasts.svelte'
  import { ask, newAsk, removeThread, reopen, resumeAsk, keepLooking, running, startAsk, stopAsk, threads, trail } from '../lib/ask.svelte'
  import { tick } from 'svelte'
  import { marked } from 'marked'
  import DOMPurify from 'dompurify'
  import type { Ask, Item } from '../lib/types'
  import Icon from './Icon.svelte'

  let {
    items,
    onselect,
    onclose,
    onscope,
    onsettings,
    suggestions,
  }: {
    items: Map<string, Item>
    /** A cited note (or a search result) was clicked. */
    /** Opens a note; for a source, with the passage it quoted. */
    onselect: (id: string, quote?: string) => void
    onclose: () => void
    /** The note or topic the questions are about was clicked: show it. */
    onscope: (scope: NonNullable<typeof ask.scope>) => void
    /** Opens the AI part of Settings. */
    onsettings: () => void
    /** Example questions for an empty panel, made from the library's own topics. */
    suggestions: string[]
  } = $props()

  let draft = $state('')
  let box = $state<HTMLTextAreaElement>()
  let scroller = $state<HTMLDivElement>()

  const current = $derived(ask.current)
  const last = $derived(ask.steps.at(-1))
  const looked = $derived(trail().length)

  /**
   * The answer as safe HTML: Markdown, with every citation mark `[1]` made a button. Links and
   * images are dropped (an answer has no business loading or opening anything), and marks inside
   * code are left alone.
   */
  function html(a: Ask): string {
    const clean = DOMPurify.sanitize(marked.parse(a.answer, { async: false, breaks: true }), { FORBID_TAGS: ['a', 'img', 'button'] })
    const page = new DOMParser().parseFromString(`<body>${clean}</body>`, 'text/html')
    const walker = page.createTreeWalker(page.body, NodeFilter.SHOW_TEXT)
    const texts: Text[] = []
    for (let node = walker.nextNode(); node; node = walker.nextNode()) if (!node.parentElement?.closest('code, pre')) texts.push(node as Text)
    for (const text of texts) {
      const marks = [...text.data.matchAll(/\[(\d{1,2}(?:\s*,\s*\d{1,2})*)\]/g)]
      if (!marks.length) continue
      const frag = page.createDocumentFragment()
      let from = 0
      for (const mark of marks) {
        frag.append(text.data.slice(from, mark.index))
        for (const n of mark[1].split(',').map((x) => Number(x.trim()))) {
          const button = page.createElement('button')
          button.className = 'cite'
          button.dataset.n = String(n)
          button.textContent = String(n)
          const source = sourceOf(a, n)
          if (source) button.title = `“${source.quote}” — ${titleOf(source.itemId)}`
          else button.disabled = true
          frag.append(button)
        }
        from = mark.index + mark[0].length
      }
      frag.append(text.data.slice(from))
      text.replaceWith(frag)
    }
    return page.body.innerHTML
  }

  /** One handler for every citation button in an answer. */
  function citeClick(event: MouseEvent, a: Ask) {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>('button.cite')
    const source = button && sourceOf(a, Number(button.dataset.n))
    if (source) onselect(source.itemId, source.quote)
  }

  const titleOf = (id: string) => items.get(id)?.title ?? 'a deleted note'
  const sourceOf = (a: Ask, n: number) => a.sources.find((s) => s.n === n)
  const conversations = $derived(threads())

  async function submit() {
    const question = draft
    if (!question.trim()) return
    draft = ''
    void startAsk(question)
    // Once the box is empty it shrinks back to one line.
    await tick()
    resize()
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
    if (Number.isNaN(date.getTime())) return ''
    const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(date).setHours(0, 0, 0, 0)) / 86_400_000)
    if (days <= 0) return 'Today'
    if (days === 1) return 'Yesterday'
    if (days < 7) return date.toLocaleDateString(undefined, { weekday: 'long' })
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      toast('Copied the answer')
    } catch {
      toast("Couldn't copy")
    }
  }

  function noteOf(a: Ask): string {
    if (a.state === 'partial') return a.limitHit === 'tokens' ? 'I ran out of room for this question, so this may be incomplete.' : 'I ran out of steps, so this may be incomplete.'
    if (a.state === 'stopped') return 'Stopped early; this is what was found so far.'
    if (a.unsupported) return "I couldn't tie this to your notes, so treat it with care."
    return ''
  }
</script>

{#snippet exchange(a: Ask, live: boolean)}
  <section class="exchange" class:earlier={!live}>
    <p class="question">{a.question}</p>
    {#if live && running()}
      <p class="progress">
        <span class="spin"></span>
        {last ? last.label[0].toUpperCase() + last.label.slice(1) : 'Starting'}…
        {#if looked > 1}<span class="muted">{looked} notes so far</span>{/if}
      </p>
    {:else if a.answer}
      {#if live && ask.steps.length}
        <details class="steps">
          <summary>Looked through {trail().length} {trail().length === 1 ? 'note' : 'notes'}</summary>
          <ol>
            {#each ask.steps as step (step.n)}<li>{step.label[0].toUpperCase() + step.label.slice(1)}</li>{/each}
          </ol>
        </details>
      {/if}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="answer" onclick={(event) => citeClick(event, a)}>{@html html(a)}</div>
      <div class="answer-tools">
        <button class="icon-btn" onclick={() => copy(a.answer)} title="Copy the answer" aria-label="Copy the answer"><Icon name="copy" size={14} /></button>
        {#if noteOf(a)}<span class="note">{noteOf(a)}</span>{/if}
        {#if live && a.limitHit}
          {#if a.state === 'partial'}<button class="btn small" onclick={keepLooking}>Keep looking</button>
          {:else}<button class="deeper" onclick={keepLooking}>Look deeper</button>{/if}
        {/if}
      </div>
      {#if a.sources.length}
        <h3>Sources</h3>
        <ol class="sources">
          {#each a.sources as source (source.n)}
            <li>
              <button onclick={() => onselect(source.itemId, source.quote)}>
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
  </section>
{/snippet}

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

  {#if ask.scope}
    <!-- What the questions are about stays in sight, whatever has been asked. -->
    <div class="anchor" role="group" aria-label="What you're asking about">
      <button class="open" onclick={() => onscope(ask.scope!)} title={ask.scope.kind === 'note' ? 'Show this note' : 'Show this topic'}>
        <Icon name={ask.scope.kind === 'note' ? 'file' : 'tree'} size={13} />
        <span>About {ask.scope.label}</span>
      </button>
      <button class="icon-btn" onclick={() => (ask.scope = null)} title="Ask about everything instead" aria-label="Ask about everything instead"><Icon name="close" size={13} /></button>
    </div>
  {/if}

  <div class="scroll" bind:this={scroller}>
    {#if ask.historyOpen}
      {#if conversations.length}
        <ul class="history">
          {#each conversations as thread (thread.id)}
            <li>
              <button onclick={() => reopen(thread.id)}>
                <span class="q">{thread.first.question}</span>
                <span class="when">{thread.count > 1 ? `${thread.count} questions · ` : ''}{when(thread.latest)}</span>
              </button>
              <button class="icon-btn" onclick={() => removeThread(thread.id)} title="Delete" aria-label="Delete"><Icon name="trash" size={14} /></button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No questions yet.</p>
      {/if}
    {:else if current}
      {#each ask.earlier as past (past.id)}
        {@render exchange(past, false)}
      {/each}
      {@render exchange(current, true)}

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
        {#if suggestions.length}
          <p class="try">Try asking</p>
          <ul class="suggestions">
            {#each suggestions as suggestion (suggestion)}
              <li><button onclick={() => startAsk(suggestion)}>{suggestion}</button></li>
            {/each}
          </ul>
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
      placeholder={current && !running() ? (ask.scope ? `Ask a follow-up about ${ask.scope.label}` : 'Ask a follow-up') : ask.scope ? `Ask about ${ask.scope.label}` : 'Ask about your notes'}
      aria-label="Question"
    ></textarea>
    {#if running()}
      <button type="button" class="icon-btn" onclick={stopAsk} title="Stop" aria-label="Stop"><Icon name="stop" /></button>
    {:else}
      <button type="submit" class="icon-btn send" disabled={!draft.trim()} title="Ask (Enter)" aria-label="Ask"><Icon name="send" /></button>
    {/if}
  </form>
  <p class="hint" class:show={draft.trim().length > 0}>Enter to ask · Shift+Enter for a new line</p>
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

  .exchange {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .exchange.earlier {
    padding-bottom: 12px;
    border-bottom: 1px solid var(--line);
  }

  .question {
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 600;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .anchor {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    margin: 0 12px 4px 16px;
    padding: 3px 4px 3px 10px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 12px;
    align-self: flex-start;
    max-width: calc(100% - 28px);
  }

  .anchor .open {
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
  }

  .anchor .open span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .anchor .icon-btn {
    width: 20px;
    height: 20px;
    color: inherit;
  }

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
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .answer :global(> :first-child) {
    margin-top: 0;
  }

  .answer :global(> :last-child) {
    margin-bottom: 0;
  }

  .answer :global(p),
  .answer :global(ul),
  .answer :global(ol),
  .answer :global(pre),
  .answer :global(blockquote) {
    margin: 0 0 8px;
  }

  .answer :global(ul),
  .answer :global(ol) {
    padding-left: 20px;
  }

  .answer :global(li) {
    margin: 2px 0;
  }

  .answer :global(h1),
  .answer :global(h2),
  .answer :global(h3),
  .answer :global(h4) {
    margin: 12px 0 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    text-transform: none;
    letter-spacing: 0;
  }

  .answer :global(code) {
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--surface-2);
    font: 12px var(--mono);
  }

  .answer :global(pre) {
    overflow-x: auto;
    padding: 10px;
    border-radius: 8px;
    background: var(--surface-2);
  }

  .answer :global(pre code) {
    padding: 0;
    background: none;
  }

  .answer :global(blockquote) {
    padding-left: 10px;
    border-left: 2px solid var(--border);
    color: var(--text-muted);
  }

  .answer :global(.cite) {
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

  .answer :global(.cite:hover:not(:disabled)) {
    background: var(--accent);
    color: white;
  }

  .answer :global(.cite:disabled) {
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
  .matches li button {
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
    min-width: 0;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    padding: 6px 8px;
    margin: 0 0 0 -8px;
    border: none;
    border-radius: 8px;
    background: none;
    text-align: left;
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

  .hint {
    flex: none;
    margin: -4px 0 0;
    padding: 0 16px 8px;
    height: 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: 11px;
    opacity: 0;
    transition: opacity 120ms;
  }

  .hint.show {
    height: auto;
    opacity: 1;
  }

  .answer-tools {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: -4px;
  }

  .deeper {
    margin-left: auto;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .deeper:hover {
    color: var(--accent);
  }

  .answer-tools .btn.small {
    height: 26px;
    padding: 0 10px;
    font-size: 12px;
    margin-left: auto;
  }

  .steps {
    color: var(--text-muted);
    font-size: 12px;
  }

  .steps summary {
    cursor: pointer;
    list-style: none;
  }

  .steps summary::-webkit-details-marker {
    display: none;
  }

  .steps summary::before {
    content: '›';
    display: inline-block;
    width: 12px;
    transition: transform 120ms;
  }

  .steps[open] summary::before {
    transform: rotate(90deg);
  }

  .steps ol {
    margin: 4px 0 0 12px;
    padding-left: 12px;
    list-style: decimal;
  }

  .try {
    margin: 12px 0 0;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .suggestions {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .suggestions button {
    width: 100%;
    padding: 8px 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--text);
    text-align: left;
  }

  .suggestions button:hover {
    background: var(--fill-hover);
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
