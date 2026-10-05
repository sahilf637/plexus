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
  import { navigation, setNavigation } from '../lib/navigation.svelte'
  import Icon, { type IconName } from './Icon.svelte'
  import Select, { type Option } from './Select.svelte'
  import type { AiConnection, AiKeyStatus, AiSettings, HotkeyStatus, ModelInfo, Settings } from '../lib/types'

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

  // AI providers. A provider is added with one form and then only chosen or removed; its key is
  // typed in the form but only ever stored in the system keychain.
  let keyStatuses = $state<Record<string, AiKeyStatus>>({})
  /** The provider form is showing: to add one, or (with `editing`) to change one. */
  let adding = $state(false)
  /** The provider being changed; its kind, address and key slot stay as they were. */
  let editing = $state<AiConnection | null>(null)
  let removing = $state<string | null>(null)

  // The form's fields.
  let presetId = $state(aiPresets[0].id)
  let keyDraft = $state('')
  let model = $state('')
  let typingModel = $state(false)
  let nameDraft = $state('')
  let models = $state<ModelInfo[]>([])
  let modelsState = $state<'idle' | 'loading' | 'failed'>('idle')
  let modelsError = $state('')
  let submit = $state<{ state: 'idle' | 'checking' | 'failed'; message: string }>({ state: 'idle', message: '' })

  const connections = $derived(settings?.ai.connections ?? [])
  const preset = $derived(aiPresets.find((p) => p.id === presetId) ?? aiPresets[0])
  const autoName = $derived.by(() => {
    const taken = new Set(connections.map((c) => c.name))
    let name = preset.label
    for (let n = 2; taken.has(name); n++) name = `${preset.label} ${n}`
    return name
  })
  const modelIds = $derived.by(() => {
    const ids = new Set(models.map((m) => m.id))
    const extra = [preset.model].filter((id) => !!id && !ids.has(id))
    return [...extra, ...ids]
  })
  const modelOptions = $derived<Option[]>([
    ...modelIds.map((id) => ({ value: id, label: id, hint: models.find((m) => m.id === id)?.vision ? 'sees images' : undefined })),
    { value: OTHER, label: 'Other…' },
  ])
  const providerOptions: Option[] = aiPresets.map((p) => ({ value: p.id, label: p.label }))
  /** The key already saved for the provider being edited can stay as it is. */
  const keyKept = $derived(!!editing && !!keyStatuses[editing.id]?.hasKey)
  const canAdd = $derived(!!model.trim() && (preset.local || keyKept || !!keyDraft.trim()) && submit.state !== 'checking')

  onMount(async () => {
    dialog?.showModal()
    // Focus the dialog itself, so no button starts out with a focus ring.
    dialog?.focus()
    ;[settings, status] = await Promise.all([getSettings(), hotkeyStatus()])
    hotkeyDraft = settings.hotkey
    await loadKeyStatuses()
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

  async function loadKeyStatuses() {
    const next: Record<string, AiKeyStatus> = {}
    await Promise.all(connections.map(async (c) => (next[c.id] = await aiKeyStatus(c.id))))
    keyStatuses = next
  }

  async function updateAi(patch: Partial<AiSettings>) {
    if (!settings) return
    await update({ ai: { ...settings.ai, ...patch } })
  }

  // --- the list ---------------------------------------------------------------------------

  function summary(c: AiConnection): string {
    return [aiPresets.find((p) => p.id === c.preset)?.label ?? c.preset, c.model].filter(Boolean).join(' · ')
  }

  /** A provider without a key can't make requests (unless it's a server on this computer). */
  function needsKey(c: AiConnection): boolean {
    return !aiPresets.find((p) => p.id === c.preset)?.local && !keyStatuses[c.id]?.hasKey
  }

  async function toggleAi(enabled: boolean) {
    await updateAi({ enabled })
    // Nothing to use yet: go straight to adding one.
    if (enabled && !connections.length) startAdding()
  }

  async function remove(id: string) {
    if (!settings) return
    const rest = connections.filter((c) => c.id !== id)
    const active = settings.ai.active === id ? (rest[0]?.id ?? '') : settings.ai.active
    removing = null
    await updateAi({ connections: rest, active })
  }

  // --- the form ---------------------------------------------------------------------------

  function startEditing(c: AiConnection) {
    presetId = aiPresets.some((p) => p.id === c.preset) ? c.preset : aiPresets[0].id
    keyDraft = ''
    nameDraft = c.name
    typingModel = false
    model = c.model
    models = []
    modelsState = 'idle'
    submit = { state: 'idle', message: '' }
    editing = c
    adding = true
    loadModels()
  }

  function startAdding() {
    editing = null
    presetId = aiPresets[0].id
    keyDraft = ''
    nameDraft = ''
    typingModel = false
    model = aiPresets[0].model
    models = []
    submit = { state: 'idle', message: '' }
    adding = true
    loadModels()
  }

  /** What the form describes, as a connection (its id is final: the key is stored under it). */
  let draftId = $state(crypto.randomUUID())
  function draft(): AiConnection {
    return {
      id: editing?.id ?? draftId,
      name: nameDraft.trim() || (editing?.name ?? autoName),
      preset: editing?.preset ?? preset.id,
      kind: editing?.kind ?? preset.kind,
      baseUrl: editing?.baseUrl ?? preset.baseUrl,
      model: model.trim(),
      vision: models.find((m) => m.id === model)?.vision ?? (editing && model === editing.model ? editing.vision : preset.vision),
    }
  }

  async function loadModels() {
    // Needs a key first (typed, or saved when editing), unless it's a server on this computer.
    if (!preset.local && !keyKept && !keyDraft.trim()) {
      models = []
      modelsState = 'idle'
      return
    }
    const wanted = preset.id
    modelsState = 'loading'
    try {
      const list = await listAiModels(draft(), keyDraft.trim() || undefined)
      if (!adding || preset.id !== wanted) return
      models = list
      modelsState = 'idle'
      // The preset's own pick if the provider has it, else its first model.
      if (!typingModel && list.length && !list.some((m) => m.id === model)) model = list[0].id
    } catch (e) {
      if (preset.id !== wanted) return
      models = []
      modelsState = 'failed'
      modelsError = describe(e)
    }
  }

  function pickPreset(id: string) {
    const next = aiPresets.find((p) => p.id === id)
    if (!next) return
    presetId = id
    keyDraft = ''
    typingModel = false
    model = next.model
    models = []
    modelsState = 'idle'
    loadModels()
  }

  function pickModel(id: string) {
    if (id === OTHER) {
      typingModel = true
      return
    }
    typingModel = false
    model = id
  }

  /** Checks the provider with what was typed (or its saved key), and only then keeps it. */
  async function add() {
    if (!settings || !canAdd) return
    const connection = draft()
    submit = { state: 'checking', message: '' }
    try {
      await testAi(connection, keyDraft.trim() || undefined)
    } catch (e) {
      submit = { state: 'failed', message: describe(e) }
      return
    }
    try {
      if (keyDraft.trim()) await setAiKey(connection.id, keyDraft)
      const next = editing ? connections.map((c) => (c.id === connection.id ? connection : c)) : [...connections, connection]
      await updateAi({ connections: next, active: settings.ai.active || connection.id })
    } catch (e) {
      submit = { state: 'failed', message: describe(e) }
      return
    }
    keyDraft = ''
    if (!editing) draftId = crypto.randomUUID()
    editing = null
    adding = false
    await loadKeyStatuses()
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
        {#if section === 'ai' && adding}
          <button class="crumb" onclick={() => (adding = false)}><Icon name="back" size={16} />{editing ? 'Edit provider' : 'Add provider'}</button>
        {:else}
          <h3>{SECTIONS.find((s) => s.id === section)?.label}</h3>
        {/if}
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

            <h4>Map</h4>
            <div class="group">
              <label class="item">
                <div class="label">
                  <span>Reverse zoom direction</span>
                  <span class="desc">On: slide two fingers up to zoom in. Off: like a mouse wheel</span>
                </div>
                <input type="checkbox" class="switch" checked={navigation.invertZoom} onchange={(e) => setNavigation({ invertZoom: e.currentTarget.checked })} />
              </label>
              <label class="item">
                <div class="label">
                  <span>Move with the cursor <span class="tag">Experimental</span></span>
                  <span class="desc">The view drifts towards the cursor, faster the further it is from the middle</span>
                </div>
                <input type="checkbox" class="switch" checked={navigation.edgeScroll} onchange={(e) => setNavigation({ edgeScroll: e.currentTarget.checked })} />
              </label>
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
          {:else if section === 'ai' && adding}
            <!-- Add a provider -->
            <div class="group">
              <div class="item">
                <div class="label"><span>Provider</span></div>
                <Select value={presetId} options={providerOptions} onchange={pickPreset} label="Provider" width={240} disabled={!!editing} />
              </div>

              {#if !preset.local}
                <div class="item">
                  <div class="label">
                    <span>Key</span>
                    <span class="desc">{keyKept ? 'Saved in your keychain. Paste a new one to replace it' : 'Kept in your system keychain, never in a file'}</span>
                  </div>
                  <!-- svelte-ignore a11y_autofocus -->
                  <input
                    class="input field"
                    class:saved={keyKept && !keyDraft}
                    type="password"
                    autocomplete="off"
                    autofocus
                    bind:value={keyDraft}
                    placeholder={keyKept ? '••••••••••••••••' : 'Paste your key'}
                    aria-label="API key"
                    onchange={loadModels}
                  />
                </div>
              {/if}

              <div class="item">
                <div class="label">
                  <span>Model</span>
                  <span class="desc">
                    {#if modelsState === 'loading'}Loading the list…
                    {:else if modelsState === 'failed'}<span class="error" title={modelsError}>Couldn't load the list</span>
                    {:else if models.length}{models.length} available
                    {:else if !preset.local && !keyKept && !keyDraft.trim()}Paste your key to see the list{/if}
                  </span>
                </div>
                {#if typingModel}
                  <!-- svelte-ignore a11y_autofocus -->
                  <input class="input field" autofocus bind:value={model} placeholder="model name" aria-label="Model name" />
                {:else}
                  <Select value={model} options={modelOptions} onchange={pickModel} label="Model" width={240} placeholder="Choose a model" />
                {/if}
              </div>

              <div class="item">
                <div class="label">
                  <span>Name</span>
                  <span class="desc">Optional, to tell providers apart</span>
                </div>
                <input class="input field" bind:value={nameDraft} placeholder={editing?.name ?? autoName} aria-label="Name" />
              </div>
            </div>

            {#if submit.state === 'failed'}
              <p class="problem" role="alert">{submit.message}</p>
            {/if}
            <div class="actions">
              <button class="btn" onclick={() => (adding = false)}>Cancel</button>
              <button class="btn primary" onclick={add} disabled={!canAdd}>
                {submit.state === 'checking' ? 'Checking…' : editing ? 'Save' : 'Add provider'}
              </button>
            </div>
          {:else if section === 'ai'}
            <!-- The providers -->
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
              <h4>Providers</h4>
              <div class="group" role="radiogroup" aria-label="Provider in use">
                {#each connections as c (c.id)}
                  <div class="item">
                    <button
                      class="row"
                      role="radio"
                      aria-checked={settings.ai.active === c.id}
                      onclick={() => updateAi({ active: c.id })}
                      title={settings.ai.active === c.id ? 'In use' : 'Use this one'}
                    >
                      <span class="radio" class:on={settings.ai.active === c.id} aria-hidden="true"></span>
                      <span class="label">
                        <span>{c.name}</span>
                        <span class="desc">
                          {summary(c)}{#if needsKey(c)}{' · '}<span class="warn">needs a key</span>{/if}
                        </span>
                      </span>
                    </button>
                    {#if removing === c.id}
                      <button class="btn" onclick={() => (removing = null)}>Keep</button>
                      <button class="btn danger" onclick={() => remove(c.id)}>Remove</button>
                    {:else}
                      {#if settings.ai.active === c.id}<span class="tag">In use</span>{/if}
                      <button class="icon-btn" onclick={() => startEditing(c)} title="Edit" aria-label={`Edit ${c.name}`}>
                        <Icon name="edit" size={15} />
                      </button>
                      <button class="icon-btn danger" onclick={() => (removing = c.id)} title="Remove (also deletes its key)" aria-label={`Remove ${c.name}`}>
                        <Icon name="trash" size={15} />
                      </button>
                    {/if}
                  </div>
                {:else}
                  <div class="item">
                    <div class="label">
                      <span>No providers yet</span>
                      <span class="desc">Add one to start using AI</span>
                    </div>
                  </div>
                {/each}
              </div>
              <div class="actions start">
                <button class="btn" onclick={startAdding}><Icon name="plus" size={15} />Add provider</button>
              </div>

              <h4>Options</h4>
              <div class="group">
                <div class="item" title="The most AI requests per day, to keep costs predictable. 0 means no limit.">
                  <div class="label"><span>Daily limit</span></div>
                  <input class="input num" type="number" min="0" step="50" value={settings.ai.dailyLimit} aria-label="Daily limit" onchange={(e) => updateAi({ dailyLimit: Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)) })} />
                </div>
              </div>
              <p class="note">Tip: use a key with a spend limit, and only the permissions Plexus needs.</p>
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
    min-height: 46px;
    padding: 8px 12px 4px 22px;
  }

  h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 650;
  }

  /* "‹ Add provider": back to the list. */
  .crumb {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: -8px;
    padding: 4px 8px;
    border: none;
    border-radius: 8px;
    background: none;
    font-size: 16px;
    font-weight: 650;
  }

  .crumb:hover {
    background: var(--hover);
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
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 56px;
    padding: 10px 14px;
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

  .warn {
    color: var(--warning);
  }

  /* A provider in the list: the name area chooses it. */
  /* A saved key is never shown, only that there is one: dots in the normal text colour. */
  .input.saved::placeholder {
    color: var(--text);
    opacity: 0.75;
    letter-spacing: 0.15em;
  }

  .row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0;
    border: none;
    background: none;
    text-align: left;
  }

  .radio {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 50%;
    box-shadow: inset 0 0 0 2px var(--fill-hover);
    transition: box-shadow 120ms;
  }

  .radio.on {
    box-shadow: inset 0 0 0 5px var(--accent);
  }

  .tag {
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }

  .actions.start {
    justify-content: flex-start;
  }

  .problem {
    margin: 12px 2px 0;
    color: var(--danger);
    overflow-wrap: anywhere;
  }

  .note {
    margin: 10px 2px 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .key {
    width: 170px;
  }

  .field {
    width: 240px;
  }

  .num {
    width: 96px;
    text-align: right;
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
