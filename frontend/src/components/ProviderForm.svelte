<script lang="ts">
  // The form that adds or changes an AI provider, in Settings and in first-run setup. The key is
  // typed here but only ever stored in the system keychain, and only once the provider answered.
  import { listAiModels, openUrl, testAi } from '../lib/api'
  import { aiPresets } from '../lib/kinds'
  import Select, { type Option } from './Select.svelte'
  import type { AiConnection, AiProblem, ModelInfo } from '../lib/types'

  let {
    connections,
    editing = null,
    keyKept = false,
    onsave,
    oncancel,
    cancelLabel = 'Cancel',
    saveLabel,
  }: {
    /** The providers already saved, so a new one gets a name of its own. */
    connections: AiConnection[]
    /** The provider being changed; its kind, address and key slot stay as they were. */
    editing?: AiConnection | null
    /** The provider being edited has a key in the keychain, so it can stay as it is. */
    keyKept?: boolean
    /** Keeps the provider once it answered; `key` is the one typed, empty to keep the saved one.
     * Rejecting shows why under the form. */
    onsave: (connection: AiConnection, key: string) => Promise<void>
    oncancel: () => void
    cancelLabel?: string
    saveLabel?: string
  } = $props()

  const OTHER = '__other__'

  // svelte-ignore state_referenced_locally: the form starts from the provider being edited
  const start = aiPresets.find((p) => p.id === editing?.preset) ?? aiPresets[0]
  let presetId = $state(start.id)
  let keyDraft = $state('')
  // svelte-ignore state_referenced_locally
  let model = $state(editing?.model ?? start.model)
  let typingModel = $state(false)
  // svelte-ignore state_referenced_locally
  let nameDraft = $state(editing?.name ?? '')
  let models = $state<ModelInfo[]>([])
  let modelsState = $state<'idle' | 'loading' | 'failed'>('idle')
  let modelsError = $state('')
  let submit = $state<{ state: 'idle' | 'checking' | 'failed'; message: string }>({ state: 'idle', message: '' })
  /** Models tried with this key that the provider wouldn't answer with, and why. */
  let failed = $state<Record<string, 'model' | 'busy'>>({})
  /** A new provider's id is final from the start: its key is stored under it. */
  const newId = crypto.randomUUID()

  const preset = $derived(aiPresets.find((p) => p.id === presetId) ?? aiPresets[0])
  const autoName = $derived.by(() => {
    const taken = new Set(connections.map((c) => c.name))
    let name = preset.label
    for (let n = 2; taken.has(name); n++) name = `${preset.label} ${n}`
    return name
  })
  // The provider's own list when it has given one; until then, just the preset's pick.
  const modelOptions = $derived<Option[]>([
    ...(models.length ? models.map((m) => m.id) : [preset.model].filter(Boolean)).map((id) => ({
      value: id,
      label: id,
      hint: hintFor(id, models.find((m) => m.id === id)?.vision ?? false),
    })),
    { value: OTHER, label: 'Other…' },
  ])
  const providerOptions: Option[] = aiPresets.map((p) => ({ value: p.id, label: p.label, hint: p.hint }))
  const canSave = $derived(!!model.trim() && (preset.local || keyKept || !!keyDraft.trim()) && submit.state !== 'checking')

  loadModels()

  function hintFor(id: string, vision: boolean): string | undefined {
    if (failed[id] === 'busy') return 'busy'
    if (failed[id]) return "didn't work"
    return vision ? 'sees images' : undefined
  }

  /** What the backend sent back as a failure: a plain sentence and what to blame. */
  function problem(e: unknown): AiProblem {
    if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
      return { kind: 'kind' in e ? (e.kind as AiProblem['kind']) : 'other', message: e.message }
    }
    const text = String(e).replace(/^Error:\s*/, '')
    return { kind: 'other', message: text.charAt(0).toUpperCase() + text.slice(1) }
  }

  /** What the form describes, as a connection. */
  function draft(): AiConnection {
    return {
      id: editing?.id ?? newId,
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
      if (preset.id !== wanted) return
      models = list
      modelsState = 'idle'
      // The preset's own pick if the provider has it, else its first model.
      if (!typingModel && list.length && !list.some((m) => m.id === model)) model = list[0].id
    } catch (e) {
      if (preset.id !== wanted) return
      models = []
      modelsState = 'failed'
      modelsError = problem(e).message
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
    failed = {}
    modelsState = 'idle'
    loadModels()
  }

  function pickModel(id: string) {
    // The last failure was about the model that was picked before.
    submit = { state: 'idle', message: '' }
    if (id === OTHER) {
      typingModel = true
      return
    }
    typingModel = false
    model = id
  }

  /** Checks the provider with what was typed (or its saved key), and only then keeps it. */
  async function save() {
    if (!canSave) return
    const connection = draft()
    submit = { state: 'checking', message: '' }
    try {
      await testAi(connection, keyDraft.trim() || undefined)
      await onsave(connection, keyDraft.trim())
      submit = { state: 'idle', message: '' }
    } catch (e) {
      const why = problem(e)
      // Only a model's own trouble is held against it, not a wrong key or a bad connection.
      if (why.kind === 'model' || why.kind === 'busy') failed[connection.model] = why.kind
      submit = { state: 'failed', message: why.message }
    }
  }
</script>

<div class="group">
  <div class="item">
    <div class="label"><span>Provider</span></div>
    <Select value={presetId} options={providerOptions} onchange={pickPreset} label="Provider" width={260} disabled={!!editing} />
  </div>

  {#if !preset.local}
    <div class="item">
      <div class="label">
        <span>Key</span>
        <span class="desc">
          {#if keyKept}Saved in your keychain. Paste a new one to replace it
          {:else}Kept in your system keychain, never in a file{#if preset.keyUrl}. <button class="inline-link" onclick={() => openUrl(preset.keyUrl!)}>Get a key</button>{/if}{/if}
        </span>
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
        onchange={() => {
          failed = {}
          loadModels()
        }}
        onkeydown={(e) => e.key === 'Enter' && save()}
      />
    </div>
  {:else}
    <div class="item">
      <div class="label">
        <span>Runs on this computer</span>
        <span class="desc">No key needed. Start {preset.label.replace(/ \(.*\)$/, '')} first, then pick a model</span>
      </div>
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
      <input
        class="input field"
        autofocus
        bind:value={model}
        placeholder="model name"
        aria-label="Model name"
        oninput={() => (submit = { state: 'idle', message: '' })}
      />
    {:else}
      <Select value={model} options={modelOptions} onchange={pickModel} label="Model" width={260} placeholder="Choose a model" />
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
  <button class="btn" onclick={oncancel}>{cancelLabel}</button>
  <button class="btn primary" onclick={save} disabled={!canSave}>
    {submit.state === 'checking' ? 'Checking…' : (saveLabel ?? (editing ? 'Save' : 'Add provider'))}
  </button>
</div>

<style>
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
</style>
