<script lang="ts">
  import { onMount } from 'svelte'
  import {
    aiKeyStatus,
    configureHotkey,
    getSettings,
    hotkeyStatus,
    listAiModels,
    setAiKey,
    setSettings,
    testAi,
  } from '../lib/api'
  import { aiPresets } from '../lib/kinds'
  import Icon, { type IconName } from './Icon.svelte'
  import type { AiKeyStatus, AiSettings, HotkeyStatus, ModelInfo, Settings } from '../lib/types'

  type Section = 'general' | 'capture' | 'ai' | 'library'

  let { onclose, section: first = 'general' }: { onclose: () => void; section?: Section } = $props()

  const SECTIONS: { id: Section; label: string; icon: IconName }[] = [
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'capture', label: 'Capture', icon: 'clipboard' },
    { id: 'ai', label: 'AI', icon: 'sparkles' },
    { id: 'library', label: 'Library', icon: 'folder' },
  ]
  const OTHER = '__other__'

  // svelte-ignore state_referenced_locally: the section it opens on; the user moves it from there
  let section = $state<Section>(first)
  let settings = $state<Settings | null>(null)
  let status = $state<HotkeyStatus | null>(null)
  let hotkeyDraft = $state('')
  let dialog = $state<HTMLDialogElement>()

  // AI: the key is typed here but only ever stored in the system keychain.
  let keyStatus = $state<AiKeyStatus>({ hasKey: false, error: null })
  let keyDraft = $state('')
  let test = $state<{ state: 'idle' | 'running' | 'ok' | 'failed'; message: string }>({ state: 'idle', message: '' })
  let models = $state<ModelInfo[]>([])
  let modelsState = $state<{ state: 'idle' | 'loading' | 'failed'; message: string }>({ state: 'idle', message: '' })
  /** The model box is a text field: the user chose "Other…". */
  let typingModel = $state(false)
  const preset = $derived(aiPresets.find((p) => p.id === settings?.ai.preset) ?? aiPresets[aiPresets.length - 1])
  /** The provider's models, plus the one in use and the preset's own, so the box is never empty. */
  const modelOptions = $derived.by(() => {
    const ids = new Set(models.map((m) => m.id))
    const extra = [settings?.ai.model, preset.model].filter((id): id is string => !!id && !ids.has(id))
    return [...new Set(extra), ...ids]
  })

  onMount(async () => {
    dialog?.showModal()
    // Focus the dialog itself, so no button starts out with a focus ring.
    dialog?.focus()
    ;[settings, status] = await Promise.all([getSettings(), hotkeyStatus()])
    hotkeyDraft = settings.hotkey
    keyStatus = await aiKeyStatus()
    if (settings.ai.enabled) loadModels()
  })

  async function update(patch: Partial<Settings>) {
    if (!settings) return
    settings = await setSettings({ ...settings, ...patch })
    // Registration is asynchronous on some systems; give it a moment before re-reading.
    setTimeout(async () => (status = await hotkeyStatus()), 300)
  }

  /** An error as a sentence: no "Error:" label, first letter capitalised. */
  function describe(e: unknown): string {
    const text = String(e).replace(/^Error:\s*/, '')
    return text.charAt(0).toUpperCase() + text.slice(1)
  }

  async function updateAi(patch: Partial<AiSettings>) {
    if (!settings) return
    test = { state: 'idle', message: '' }
    await update({ ai: { ...settings.ai, ...patch } })
  }

  async function loadModels() {
    modelsState = { state: 'loading', message: '' }
    try {
      models = await listAiModels()
      modelsState = { state: 'idle', message: '' }
    } catch (e) {
      models = []
      modelsState = { state: 'failed', message: describe(e) }
    }
  }

  async function toggleAi(enabled: boolean) {
    await updateAi({ enabled })
    if (enabled) loadModels()
  }

  async function pickPreset(id: string) {
    const next = aiPresets.find((p) => p.id === id)
    if (!next) return
    keyDraft = ''
    typingModel = false
    models = []
    await updateAi({ preset: next.id, kind: next.kind, baseUrl: next.baseUrl, model: next.model, vision: next.vision })
    keyStatus = await aiKeyStatus()
    loadModels()
  }

  async function pickModel(id: string) {
    if (id === OTHER) {
      typingModel = true
      return
    }
    typingModel = false
    // The provider says whether it reads images; follow that.
    const known = models.find((m) => m.id === id)
    await updateAi({ model: id, ...(known ? { vision: known.vision } : {}) })
  }

  async function saveKey() {
    if (!keyDraft.trim()) return
    try {
      await setAiKey(keyDraft)
      keyDraft = ''
    } catch (e) {
      test = { state: 'failed', message: describe(e) }
    }
    keyStatus = await aiKeyStatus()
    loadModels()
  }

  async function runTest() {
    await saveKey()
    test = { state: 'running', message: '' }
    try {
      await testAi()
      test = { state: 'ok', message: 'Connected' }
    } catch (e) {
      test = { state: 'failed', message: describe(e) }
    }
  }
</script>

<dialog bind:this={dialog} {onclose} onclick={(e) => e.target === dialog && dialog?.close()}>
  <div class="shell">
    <nav aria-label="Settings sections">
      <h2>Settings</h2>
      {#each SECTIONS as s}
        <button class:on={section === s.id} aria-current={section === s.id ? 'page' : undefined} onclick={() => (section = s.id)}>
          <Icon name={s.icon} size={16} />{s.label}
        </button>
      {/each}
    </nav>

    <div class="pane">
      <header>
        <h3>{SECTIONS.find((s) => s.id === section)?.label}</h3>
        <button class="icon-btn" onclick={() => dialog?.close()} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
      </header>

      {#if settings && status}
        <div class="body">
          {#if section === 'general'}
            <h4>Shortcut</h4>
            <div class="group">
              <div class="item" title="No global shortcut on your system? Add a custom one in its settings that runs: plexus --capture">
                <div class="label">
                  <span>Shortcut</span>
                  <span class="desc">
                    {#if status.error}<span class="error">{status.error}</span>
                    {:else if status.backend === 'portal'}Set by your desktop
                    {:else}Works from any app{/if}
                  </span>
                </div>
                {#if status.trigger && !status.error}<kbd>{status.trigger}</kbd>{/if}
                {#if status.backend === 'portal' || (status.backend === 'none' && status.systemConfigurable)}
                  {#if status.systemConfigurable}
                    <button class="btn" onclick={async () => { await configureHotkey(); setTimeout(async () => (status = await hotkeyStatus()), 1000) }}>
                      Change
                    </button>
                  {/if}
                {:else}
                  <input class="input key" bind:value={hotkeyDraft} aria-label="Shortcut" placeholder="Ctrl+Alt+V" />
                  <button class="btn" onclick={() => update({ hotkey: hotkeyDraft.trim() })} disabled={hotkeyDraft.trim() === settings.hotkey}>
                    Save
                  </button>
                {/if}
              </div>

              <div class="item">
                <div class="label"><span>When pressed</span></div>
                <div class="segmented" role="radiogroup" aria-label="When the shortcut is pressed">
                  <button role="radio" aria-checked={settings.hotkeyAction === 'openCapture'} class:on={settings.hotkeyAction === 'openCapture'} onclick={() => update({ hotkeyAction: 'openCapture' })}>Open window</button>
                  <button role="radio" aria-checked={settings.hotkeyAction === 'saveClipboard'} class:on={settings.hotkeyAction === 'saveClipboard'} onclick={() => update({ hotkeyAction: 'saveClipboard' })}>Save clipboard</button>
                </div>
              </div>

            </div>

            <h4>Startup</h4>
            <div class="group">
              <label class="item">
                <div class="label">
                  <span>Start at login</span>
                  <span class="desc">Stay in the tray so the shortcut always works</span>
                </div>
                <input type="checkbox" class="switch" checked={settings.startAtLogin} onchange={(e) => update({ startAtLogin: e.currentTarget.checked })} />
              </label>
            </div>
          {:else if section === 'capture'}
            <h4>Links</h4>
            <div class="group">
              <label class="item">
                <div class="label">
                  <span>Read linked pages</span>
                  <span class="desc">Fetch each saved link's page so it connects by content</span>
                </div>
                <input type="checkbox" class="switch" checked={settings.fetchLinks} onchange={(e) => update({ fetchLinks: e.currentTarget.checked })} />
              </label>
            </div>

            <h4>Clipboard</h4>
            <div class="group">
              <label class="item" title="Skips short copies and anything that looks like a password or key. Some Linux desktops (GNOME on Wayland) don't allow reading the clipboard in the background.">
                <div class="label">
                  <span>Save what I copy</span>
                  <span class="desc">Watch the clipboard, no pasting needed</span>
                </div>
                <input type="checkbox" class="switch" checked={settings.watchClipboard} onchange={(e) => update({ watchClipboard: e.currentTarget.checked })} />
              </label>
            </div>
          {:else if section === 'ai'}
            <div class="group">
              <label class="item">
                <div class="label">
                  <span>Use AI</span>
                  <span class="desc">Titles, summaries and smarter links for new items. Optional.</span>
                </div>
                <input type="checkbox" class="switch" checked={settings.ai.enabled} onchange={(e) => toggleAi(e.currentTarget.checked)} />
              </label>
            </div>

            {#if settings.ai.enabled}
              <h4>Connection</h4>
              <div class="group">
                <div class="item">
                  <div class="label"><span>Provider</span></div>
                  <select class="input pick" value={settings.ai.preset} onchange={(e) => pickPreset(e.currentTarget.value)} aria-label="Provider">
                    {#each aiPresets as p}<option value={p.id}>{p.label}</option>{/each}
                  </select>
                </div>

                {#if !preset.local}
                  <div class="item">
                    <div class="label">
                      <span>Key</span>
                      <span class="desc">
                        {#if keyStatus.error}<span class="error">{keyStatus.error}</span>
                        {:else if keyStatus.hasKey}Saved in your system keychain
                        {:else}Kept in your system keychain, never in a file{/if}
                      </span>
                    </div>
                    <input
                      class="input key"
                      type="password"
                      autocomplete="off"
                      bind:value={keyDraft}
                      placeholder={keyStatus.hasKey ? '••••••••' : 'Paste your key'}
                      aria-label="API key"
                      onchange={saveKey}
                    />
                  </div>
                {/if}

                {#if preset.custom}
                  <div class="item">
                    <div class="label"><span>Address</span></div>
                    <input class="input wide" value={settings.ai.baseUrl} placeholder="http://localhost:11434/v1" aria-label="Address" onchange={(e) => { updateAi({ baseUrl: e.currentTarget.value.trim() }).then(loadModels) }} />
                  </div>
                {/if}

                <div class="item">
                  <div class="label">
                    <span>Model</span>
                    <span class="desc">
                      {#if modelsState.state === 'loading'}Loading the list…
                      {:else if modelsState.state === 'failed'}<span class="error" title={modelsState.message}>Couldn't load the list</span>
                      {:else if models.length}{models.length} available{/if}
                    </span>
                  </div>
                  {#if typingModel}
                    <!-- svelte-ignore a11y_autofocus -->
                    <input class="input wide" autofocus value={settings.ai.model} placeholder="model name" aria-label="Model name" onchange={(e) => updateAi({ model: e.currentTarget.value.trim() })} />
                  {:else}
                    <select class="input wide" value={settings.ai.model} onchange={(e) => pickModel(e.currentTarget.value)} aria-label="Model">
                      {#each modelOptions as id}
                        <option value={id}>{id}{models.find((m) => m.id === id)?.vision ? ' · sees images' : ''}</option>
                      {/each}
                      <option value={OTHER}>Other…</option>
                    </select>
                  {/if}
                  <button class="icon-btn" onclick={loadModels} title="Reload the list" aria-label="Reload the list" disabled={modelsState.state === 'loading'}>
                    <Icon name="refresh" />
                  </button>
                </div>

                <div class="item">
                  <div class="label">
                    <span class:ok={test.state === 'ok'} class:error={test.state === 'failed'} class="result">
                      {test.state === 'running' ? 'Checking…' : test.message || 'Not checked yet'}
                    </span>
                  </div>
                  <button class="btn" onclick={runTest} disabled={test.state === 'running' || !settings.ai.model}>Test connection</button>
                </div>
              </div>

              <h4>Options</h4>
              <div class="group">
                <label class="item">
                  <div class="label">
                    <span>Describe images</span>
                    <span class="desc">Needs a model that can see pictures</span>
                  </div>
                  <input type="checkbox" class="switch" checked={settings.ai.vision} onchange={(e) => updateAi({ vision: e.currentTarget.checked })} />
                </label>

                <div class="item" title="The most AI requests per day, to keep costs predictable. 0 means no limit.">
                  <div class="label"><span>Daily limit</span></div>
                  <input class="input num" type="number" min="0" step="50" value={settings.ai.dailyLimit} aria-label="Daily limit" onchange={(e) => updateAi({ dailyLimit: Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)) })} />
                </div>
              </div>
            {/if}
          {:else}
            <h4>Folder</h4>
            <div class="group">
              <div class="item">
                <div class="label">
                  <span>Library folder</span>
                  <span class="desc path" title={settings.libraryPath}>{settings.libraryPath}</span>
                </div>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</dialog>

<style>
  dialog {
    width: min(760px, calc(100vw - 32px));
    height: min(560px, calc(100vh - 48px));
    padding: 0;
    border: none;
    border-radius: 16px;
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  dialog:focus-visible {
    outline: none;
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.4);
    backdrop-filter: blur(2px);
  }

  .shell {
    display: grid;
    grid-template-columns: 188px 1fr;
    height: 100%;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 16px 10px;
    background: var(--bg);
  }

  h2 {
    margin: 0 0 10px;
    padding: 0 10px;
    font-size: 16px;
    font-weight: 650;
  }

  nav button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border: none;
    border-radius: 8px;
    background: none;
    color: var(--text-muted);
    text-align: left;
  }

  nav button:hover {
    background: var(--hover);
    color: var(--text);
  }

  nav button.on {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 500;
  }

  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 4px 22px;
  }

  h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 650;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 22px 22px;
  }

  h4 {
    margin: 22px 0 8px 2px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  h4:first-child {
    margin-top: 6px;
  }

  .group {
    border-radius: 12px;
    background: var(--group);
  }


  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 52px;
    padding: 8px 14px;
  }

  .item {
    position: relative;
  }

  /* A faint hairline, inset from the edges, between rows. */
  .item + .item::before {
    content: '';
    position: absolute;
    top: 0;
    left: 14px;
    right: 14px;
    height: 1px;
    background: var(--line);
  }

  label.item {
    cursor: pointer;
  }

  .label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .desc {
    color: var(--text-muted);
    font-size: 12px;
  }

  .path {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .error {
    color: var(--danger);
  }

  .key {
    width: 170px;
  }

  .pick {
    width: 200px;
  }

  .wide {
    width: 240px;
  }

  .num {
    width: 90px;
  }

  .result.ok {
    color: var(--kind-code);
  }

  .result {
    overflow-wrap: anywhere;
  }

  @media (max-width: 560px) {
    .shell {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }

    nav {
      flex-direction: row;
      overflow-x: auto;
    }

    h2 {
      display: none;
    }
  }
</style>
