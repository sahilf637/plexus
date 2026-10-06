<script lang="ts">
  // First-run setup: a few short steps over the whole window. Every step can be skipped, and
  // nothing is required: Plexus works with the defaults and without an AI provider.
  import { onMount } from 'svelte'
  import {
    configureHotkey,
    getSettings,
    hotkeyStatus,
    inspectFolder,
    pickFolder,
    restartApp,
    setAiKey,
    setSettings,
    startHotkey,
  } from '../lib/api'
  import { aiPresets } from '../lib/kinds'
  import Icon, { type IconName } from './Icon.svelte'
  import ProviderForm from './ProviderForm.svelte'
  import type { AiConnection, FolderInfo, HotkeyStatus, Settings } from '../lib/types'

  let { onfinish }: { onfinish: () => void } = $props()

  const STEPS = ['welcome', 'library', 'ai', 'shortcut', 'done'] as const
  type Step = (typeof STEPS)[number]
  let step = $state<Step>('welcome')
  const index = $derived(STEPS.indexOf(step))

  let settings = $state<Settings | null>(null)
  /** The folder chosen for the library, and what is in it. Saved only when setup ends. */
  let folder = $state<FolderInfo | null>(null)
  let picking = $state(false)
  const folderChanged = $derived(!!settings && !!folder && folder.path !== settings.libraryPath)

  /** The AI step is showing the provider form. */
  let adding = $state(false)
  const connection = $derived(settings?.ai.connections.find((c) => c.id === settings?.ai.active) ?? null)

  let hotkey = $state<HotkeyStatus | null>(null)
  let hotkeyDraft = $state('')
  let finishing = $state(false)

  onMount(async () => {
    settings = await getSettings()
    hotkeyDraft = settings.hotkey
    folder = await inspectFolder(settings.libraryPath)
  })

  async function update(patch: Partial<Settings>) {
    if (settings) settings = await setSettings({ ...settings, ...patch })
  }

  function go(next: Step) {
    step = next
    if (next === 'shortcut') turnOnShortcut()
  }

  const next = () => go(STEPS[Math.min(index + 1, STEPS.length - 1)])
  const back = () => go(STEPS[Math.max(index - 1, 0)])

  // --- library ---------------------------------------------------------------------------

  async function chooseFolder() {
    picking = true
    try {
      const path = await pickFolder()
      if (path) folder = await inspectFolder(path)
    } finally {
      picking = false
    }
  }

  async function useDefault() {
    if (settings) folder = await inspectFolder(settings.libraryPath)
  }

  /** A home-relative path reads better: ~/Plexus. */
  function shortPath(path: string): string {
    return path.replace(/^\/home\/[^/]+|^\/Users\/[^/]+|^[A-Z]:\\Users\\[^\\]+/, '~')
  }

  // --- AI --------------------------------------------------------------------------------

  /** Keeps the provider the form checked, makes it the one in use and turns on improving notes. */
  async function connect(c: AiConnection, key: string) {
    if (!settings) return
    if (key) await setAiKey(c.id, key)
    const rest = settings.ai.connections.filter((other) => other.id !== c.id)
    await update({ ai: { ...settings.ai, connections: [...rest, c], active: c.id, enabled: true } })
    adding = false
  }

  function presetLabel(c: AiConnection): string {
    return aiPresets.find((p) => p.id === c.preset)?.label ?? c.preset
  }

  // --- shortcut --------------------------------------------------------------------------

  let polling: ReturnType<typeof setInterval> | undefined

  /** Registers the shortcut and watches until it's bound or failed (on Wayland the desktop asks
   * the user first, so this can take a while). */
  async function turnOnShortcut() {
    await startHotkey()
    watchShortcut()
  }

  function watchShortcut() {
    clearInterval(polling)
    let tries = 0
    const check = async () => {
      hotkey = await hotkeyStatus()
      if (hotkey.trigger || hotkey.error || ++tries > 60) clearInterval(polling)
    }
    void check()
    polling = setInterval(check, 700)
  }

  $effect(() => () => clearInterval(polling))

  async function saveHotkey() {
    await update({ hotkey: hotkeyDraft.trim() })
    setTimeout(watchShortcut, 300)
  }

  /** The bound keys as separate keycaps when they read like "Ctrl+Alt+V". */
  const keycaps = $derived(hotkey?.trigger && /^[\w+]+$/.test(hotkey.trigger) ? hotkey.trigger.split('+') : null)

  // --- the end ---------------------------------------------------------------------------

  /** Setup is over (finished or skipped). A new library folder needs a restart to open. */
  async function finish(keepFolder: boolean) {
    if (!settings || finishing) return
    finishing = true
    const restart = keepFolder && folderChanged
    await update({ setupDone: true, ...(restart && folder ? { libraryPath: folder.path } : {}) })
    if (restart) await restartApp()
    else onfinish()
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' || event.defaultPrevented) return
    if (event.target instanceof Element && event.target.closest('input, button, [role="listbox"]')) return
    if (step === 'done') void finish(true)
    else if (step !== 'ai' || !adding) next()
  }

  const features: { icon: IconName; title: string; text: string }[] = [
    { icon: 'clipboard', title: 'Save anything', text: 'Paste text, links, screenshots or files. No folders to pick, no tags to add.' },
    { icon: 'link', title: 'It connects itself', text: 'Related things find each other on the map, and every link says why.' },
    { icon: 'sparkles', title: 'Explore and ask', text: 'Browse your notes by topic, or ask a question and get an answer from them.' },
  ]
</script>

<svelte:window {onkeydown} />

<div class="setup" role="dialog" aria-modal="true" aria-label="Set up Plexus">
  <div class="card">
    <header>
      <span class="brand"><span class="mark" aria-hidden="true"></span>Plexus</span>
      {#if step !== 'welcome'}
        <ol class="progress" aria-label="Setup steps">
          {#each STEPS.slice(1) as s, i}
            <li class:done={i + 1 < index} class:on={s === step} aria-current={s === step ? 'step' : undefined}></li>
          {/each}
        </ol>
      {/if}
      {#if step !== 'done'}
        <button class="skip" onclick={() => finish(false)} disabled={finishing}>Skip setup</button>
      {/if}
    </header>

    <div class="body prefs">
      {#if !settings}
        <!-- Settings load in a moment. -->
      {:else if step === 'welcome'}
        <div class="hero">
          <svg viewBox="0 0 200 110" aria-hidden="true">
            <g class="edges" stroke="currentColor" stroke-width="1.5">
              <line x1="30" y1="70" x2="78" y2="30" />
              <line x1="78" y1="30" x2="140" y2="42" />
              <line x1="78" y1="30" x2="100" y2="86" />
              <line x1="140" y1="42" x2="100" y2="86" />
              <line x1="140" y1="42" x2="176" y2="80" stroke-dasharray="3 4" />
              <line x1="30" y1="70" x2="100" y2="86" stroke-dasharray="3 4" />
            </g>
            <g fill="currentColor">
              <circle cx="30" cy="70" r="6" opacity="0.5" />
              <circle class="pulse" cx="78" cy="30" r="10" />
              <circle cx="140" cy="42" r="7" opacity="0.75" />
              <circle cx="100" cy="86" r="7.5" opacity="0.6" />
              <circle cx="176" cy="80" r="5" opacity="0.3" />
            </g>
          </svg>
          <h1>Copy anything. Plexus connects it.</h1>
          <p class="lead">A map of everything you save, that builds itself.</p>
        </div>
        <ul class="features">
          {#each features as f}
            <li>
              <span class="icon"><Icon name={f.icon} size={18} /></span>
              <span><strong>{f.title}</strong><span class="desc">{f.text}</span></span>
            </li>
          {/each}
        </ul>
      {:else if step === 'library'}
        <h2>Where should your notes live?</h2>
        <p class="lead">Every note is a plain file in this folder. Open it, back it up or sync it like any other folder.</p>
        <div class="group">
          <div class="item">
            <span class="icon"><Icon name="folder" size={18} /></span>
            <div class="label">
              <span class="path" title={folder?.path}>{folder ? shortPath(folder.path) : ''}</span>
              <span class="desc">
                {#if !folder}…
                {:else if folder.notes}{folder.notes} {folder.notes === 1 ? 'note' : 'notes'} already here
                {:else}Empty: a new library starts here{/if}
              </span>
            </div>
            <button class="btn" onclick={chooseFolder} disabled={picking}>{picking ? 'Choosing…' : 'Choose another folder…'}</button>
          </div>
        </div>
        {#if folderChanged}
          <p class="note">
            Plexus will restart to open this folder when setup ends.
            <button class="inline-link" onclick={useDefault}>Keep {settings ? shortPath(settings.libraryPath) : ''}</button>
          </p>
        {/if}
      {:else if step === 'ai'}
        <h2>Connect an AI <span class="optional">optional</span></h2>
        <p class="lead">
          Plexus saves, links and groups everything on this computer without one. An AI adds better titles,
          summaries, explained links, and answers to your questions.
        </p>
        {#if connection && !adding}
          <div class="group">
            <div class="item">
              <span class="icon ok"><Icon name="check" size={18} /></span>
              <div class="label">
                <span>Connected to {connection.name}</span>
                <span class="desc">{presetLabel(connection)} · {connection.model}</span>
              </div>
              <button class="btn" onclick={() => (adding = true)}>Use another</button>
            </div>
            <label class="item">
              <div class="label">
                <span>Improve new notes</span>
                <span class="desc">Titles, summaries and smarter links for what you save</span>
              </div>
              <input type="checkbox" class="switch" checked={settings.ai.enabled} onchange={(e) => update({ ai: { ...settings!.ai, enabled: e.currentTarget.checked } })} />
            </label>
          </div>
          <p class="note"><Icon name="lock" size={13} /><span>Each request sends only the note it's about and a few short summaries, never your whole library.</span></p>
        {:else if adding}
          <ProviderForm connections={settings.ai.connections} onsave={connect} oncancel={() => (adding = false)} cancelLabel="Back" saveLabel="Connect" />
        {:else}
          <div class="choices">
            <button class="choice" onclick={next}>
              <span class="icon"><Icon name="lock" size={18} /></span>
              <strong>Use without AI</strong>
              <span class="desc">Nothing leaves this computer. You can connect one later in Settings.</span>
            </button>
            <button class="choice" onclick={() => (adding = true)}>
              <span class="icon"><Icon name="sparkles" size={18} /></span>
              <strong>Connect a provider</strong>
              <span class="desc">Use your own key: OpenAI, Anthropic, Gemini, Groq, or a model running on this computer.</span>
            </button>
          </div>
        {/if}
      {:else if step === 'shortcut'}
        <h2>Add from anywhere</h2>
        <p class="lead">Press the shortcut in any app, paste, and press Enter. That's all saving takes.</p>
        <div class="keys" aria-live="polite">
          {#if keycaps}
            {#each keycaps as key, i}{#if i > 0}<span class="plus">+</span>{/if}<kbd>{key}</kbd>{/each}
          {:else if hotkey?.trigger}
            <kbd>{hotkey.trigger}</kbd>
          {:else if hotkey?.error}
            <span class="error">{hotkey.error}</span>
          {:else}
            <span class="waiting"><span class="spin" aria-hidden="true"></span>Turning on the shortcut… your desktop may ask you to allow it</span>
          {/if}
        </div>
        {#if hotkey?.error}
          <p class="note center">You can also add a shortcut in your system settings that runs <code>plexus --capture</code>.</p>
        {:else if hotkey?.trigger}
          <p class="note center">Try it now: the add window opens.</p>
        {/if}
        <div class="group">
          {#if hotkey?.systemConfigurable}
            <div class="item">
              <div class="label">
                <span>Shortcut</span>
                <span class="desc">Set by your desktop</span>
              </div>
              <button class="btn" onclick={async () => { await configureHotkey(); setTimeout(watchShortcut, 1000) }}>Change</button>
            </div>
          {:else if hotkey && (hotkey.trigger || hotkey.error) && hotkey.backend !== 'portal'}
            <div class="item">
              <div class="label">
                <span>Shortcut</span>
                <span class="desc">Works from any app</span>
              </div>
              <input class="input key" bind:value={hotkeyDraft} aria-label="Shortcut" placeholder="Ctrl+Alt+V" onkeydown={(e) => e.key === 'Enter' && saveHotkey()} />
              <button class="btn" onclick={saveHotkey} disabled={hotkeyDraft.trim() === settings.hotkey}>Save</button>
            </div>
          {/if}
          <div class="item">
            <div class="label"><span>When pressed</span></div>
            <div class="segmented" role="radiogroup" aria-label="When the shortcut is pressed">
              <button role="radio" aria-checked={settings.hotkeyAction === 'openCapture'} class:on={settings.hotkeyAction === 'openCapture'} onclick={() => update({ hotkeyAction: 'openCapture' })}>Open window</button>
              <button role="radio" aria-checked={settings.hotkeyAction === 'saveClipboard'} class:on={settings.hotkeyAction === 'saveClipboard'} onclick={() => update({ hotkeyAction: 'saveClipboard' })}>Save clipboard</button>
            </div>
          </div>
          <label class="item">
            <div class="label">
              <span>Start at login</span>
              <span class="desc">Stay in the tray so the shortcut always works</span>
            </div>
            <input type="checkbox" class="switch" checked={settings.startAtLogin} onchange={(e) => update({ startAtLogin: e.currentTarget.checked })} />
          </label>
          <label class="item" title="Skips short copies and anything that looks like a password or key.">
            <div class="label">
              <span>Save what I copy</span>
              <span class="desc">Watch the clipboard, no pasting needed. Off is the safer choice</span>
            </div>
            <input type="checkbox" class="switch" checked={settings.watchClipboard} onchange={(e) => update({ watchClipboard: e.currentTarget.checked })} />
          </label>
        </div>
      {:else}
        <div class="hero">
          <span class="done-mark"><Icon name="check" size={28} /></span>
          <h1>You're ready</h1>
          <p class="lead">Three ways to add something:</p>
        </div>
        <ul class="features">
          <li>
            <span class="icon"><Icon name="keyboard" size={18} /></span>
            <span><strong>From any app</strong><span class="desc">Press {#if hotkey?.trigger}<kbd>{hotkey.trigger}</kbd>{:else}the shortcut{/if}, paste, then Enter</span></span>
          </li>
          <li>
            <span class="icon"><Icon name="clipboard" size={18} /></span>
            <span><strong>On the map</strong><span class="desc">Press <kbd>Ctrl</kbd> <kbd>V</kbd> anywhere in the window</span></span>
          </li>
          <li>
            <span class="icon"><Icon name="file" size={18} /></span>
            <span><strong>Files</strong><span class="desc">Drop them on the add window: PDFs, documents, images</span></span>
          </li>
        </ul>
      {/if}
    </div>

    <footer>
      {#if index > 0 && !(step === 'ai' && adding)}
        <button class="btn ghost" onclick={back}><Icon name="back" size={15} />Back</button>
      {/if}
      <span class="gap"></span>
      {#if step === 'welcome'}
        <button class="btn primary big" onclick={next}>Get started</button>
      {:else if step === 'done'}
        <button class="btn primary big" onclick={() => finish(true)} disabled={finishing}>
          {folderChanged ? 'Restart and open the library' : 'Start using Plexus'}
        </button>
      {:else if step === 'ai' && adding}
        <!-- The form has its own buttons. -->
      {:else if step === 'ai' && !connection}
        <button class="btn" onclick={next}>Not now</button>
      {:else}
        <button class="btn primary big" onclick={next}>Continue</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .setup {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px 16px;
    overflow-y: auto;
    background:
      radial-gradient(1200px 600px at 20% -10%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%),
      radial-gradient(900px 500px at 110% 110%, color-mix(in srgb, var(--kind-url) 10%, transparent), transparent 70%),
      var(--bg);
    animation: fade 200ms ease-out;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .card {
    width: min(600px, 100%);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 16px 20px 0;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 650;
  }

  .mark {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: radial-gradient(circle at 30% 30%, color-mix(in srgb, var(--accent) 60%, white), var(--accent));
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .progress {
    flex: 1;
    display: flex;
    justify-content: center;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .progress li {
    width: 22px;
    height: 4px;
    border-radius: 2px;
    background: var(--fill-hover);
    transition: background 200ms, width 200ms;
  }

  .progress li.done {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }

  .progress li.on {
    width: 34px;
    background: var(--accent);
  }

  .skip {
    margin-left: auto;
    padding: 4px 6px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 13px;
  }

  .skip:hover {
    color: var(--text);
  }

  .body {
    min-height: 340px;
    padding: 22px 28px 8px;
  }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .hero svg {
    width: 200px;
    color: var(--accent);
  }

  .edges {
    opacity: 0.35;
  }

  .pulse {
    transform-origin: 78px 30px;
    animation: pulse 2.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      transform: scale(1.18);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse,
    .setup {
      animation: none;
    }
  }

  h1 {
    margin: 14px 0 0;
    font-size: 24px;
    font-weight: 650;
    letter-spacing: -0.015em;
  }

  h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .optional {
    margin-left: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--fill);
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0;
    vertical-align: middle;
  }

  .lead {
    margin: 6px 0 18px;
    color: var(--text-muted);
  }

  .hero .lead {
    margin-bottom: 22px;
  }

  .features {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin: 0 auto;
    padding: 0;
    max-width: 440px;
    list-style: none;
  }

  .features li {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }

  .features li > span:last-child {
    display: flex;
    flex-direction: column;
  }

  .icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .icon.ok {
    background: color-mix(in srgb, var(--kind-code) 16%, transparent);
    color: var(--kind-code);
  }

  .path {
    font-family: var(--mono);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 12px 2px 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .note.center {
    justify-content: center;
    margin: 0 0 18px;
  }

  .note code {
    font: 12px var(--mono);
  }

  .inline-link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-size: inherit;
  }

  .inline-link:hover {
    text-decoration: underline;
  }

  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .choice {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 16px;
    border: none;
    border-radius: 14px;
    background: var(--group);
    text-align: left;
    transition: box-shadow 120ms, background 120ms;
  }

  .choice:hover {
    background: var(--fill);
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .choice .icon {
    margin-bottom: 4px;
  }

  .keys {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 6px;
    min-height: 56px;
    margin: 4px 0 10px;
    text-align: center;
  }

  .keys kbd {
    min-width: 44px;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--surface);
    box-shadow: inset 0 -2px 0 var(--fill-hover), 0 0 0 1px var(--border);
    color: var(--text);
    font: 600 16px var(--font);
  }

  .plus {
    color: var(--text-muted);
  }

  .waiting {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
  }

  .spin {
    width: 12px;
    height: 12px;
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

  .error {
    color: var(--danger);
  }

  .key {
    width: 150px;
  }

  .done-mark {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--kind-code) 16%, transparent);
    color: var(--kind-code);
  }

  .hero h1:first-child,
  .done-mark + h1 {
    margin-top: 14px;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 68px;
    padding: 14px 20px 18px;
  }

  .gap {
    flex: 1;
  }

  .btn.ghost {
    background: none;
    color: var(--text-muted);
  }

  .btn.ghost:hover {
    color: var(--text);
  }

  .btn.big {
    height: 36px;
    padding: 0 18px;
    border-radius: 10px;
  }

  /* The provider form's own buttons sit at the bottom of the step. */
  .body :global(.actions) {
    margin-bottom: 8px;
  }

  @media (max-width: 520px) {
    .choices {
      grid-template-columns: 1fr;
    }

    .body {
      padding: 18px 18px 8px;
    }
  }
</style>
