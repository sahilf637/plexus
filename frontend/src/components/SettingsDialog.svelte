<script lang="ts">
  import { onMount } from 'svelte'
  import { aiKeyStatus, configureHotkey, getSettings, hotkeyStatus, setAiKey, setSettings, testAi } from '../lib/api'
  import { aiPresets } from '../lib/kinds'
  import Icon from './Icon.svelte'
  import type { AiKeyStatus, AiSettings, HotkeyStatus, Settings } from '../lib/types'

  let { onclose }: { onclose: () => void } = $props()

  let settings = $state<Settings | null>(null)
  let status = $state<HotkeyStatus | null>(null)
  let hotkeyDraft = $state('')
  let dialog = $state<HTMLDialogElement>()

  // AI: the key is typed here but only ever stored in the system keychain.
  let keyStatus = $state<AiKeyStatus>({ hasKey: false, error: null })
  let keyDraft = $state('')
  let test = $state<{ state: 'idle' | 'running' | 'ok' | 'failed'; message: string }>({ state: 'idle', message: '' })
  const preset = $derived(aiPresets.find((p) => p.id === settings?.ai.preset) ?? aiPresets[aiPresets.length - 1])

  onMount(async () => {
    dialog?.showModal()
    ;[settings, status] = await Promise.all([getSettings(), hotkeyStatus()])
    hotkeyDraft = settings.hotkey
    keyStatus = await aiKeyStatus()
  })

  async function updateAi(patch: Partial<AiSettings>) {
    if (!settings) return
    test = { state: 'idle', message: '' }
    await update({ ai: { ...settings.ai, ...patch } })
  }

  async function pickPreset(id: string) {
    const next = aiPresets.find((p) => p.id === id)
    if (!next) return
    keyDraft = ''
    await updateAi({ preset: next.id, kind: next.kind, baseUrl: next.baseUrl, model: next.model, vision: next.vision })
    keyStatus = await aiKeyStatus()
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
  }

  /** An error as a sentence: no "Error:" label, first letter capitalised. */
  function describe(e: unknown): string {
    const text = String(e).replace(/^Error:\s*/, '')
    return text.charAt(0).toUpperCase() + text.slice(1)
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

  async function update(patch: Partial<Settings>) {
    if (!settings) return
    settings = await setSettings({ ...settings, ...patch })
    // Registration is asynchronous on some systems; give it a moment before re-reading.
    setTimeout(async () => (status = await hotkeyStatus()), 300)
  }
</script>

<dialog bind:this={dialog} {onclose} onclick={(e) => e.target === dialog && dialog?.close()}>
  <header>
    <h2>Settings</h2>
    <button class="icon-btn" onclick={() => dialog?.close()} title="Close (Esc)" aria-label="Close"><Icon name="close" /></button>
  </header>

  {#if settings && status}
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
        <div class="label">
          <span>When pressed</span>
        </div>
        <div class="segmented" role="radiogroup" aria-label="When the shortcut is pressed">
          <button role="radio" aria-checked={settings.hotkeyAction === 'openCapture'} class:on={settings.hotkeyAction === 'openCapture'} onclick={() => update({ hotkeyAction: 'openCapture' })}>Open window</button>
          <button role="radio" aria-checked={settings.hotkeyAction === 'saveClipboard'} class:on={settings.hotkeyAction === 'saveClipboard'} onclick={() => update({ hotkeyAction: 'saveClipboard' })}>Save clipboard</button>
        </div>
      </div>
    </div>

    <div class="group">
      <label class="item">
        <div class="label">
          <span>Start at login</span>
          <span class="desc">Stay in the tray so the shortcut always works</span>
        </div>
        <input type="checkbox" class="switch" checked={settings.startAtLogin} onchange={(e) => update({ startAtLogin: e.currentTarget.checked })} />
      </label>

      <label class="item">
        <div class="label">
          <span>Read linked pages</span>
          <span class="desc">Fetch each saved link's page so it connects by content</span>
        </div>
        <input type="checkbox" class="switch" checked={settings.fetchLinks} onchange={(e) => update({ fetchLinks: e.currentTarget.checked })} />
      </label>

      <label class="item" title="Skips short copies and anything that looks like a password or key. Some Linux desktops (GNOME on Wayland) don't allow reading the clipboard in the background.">
        <div class="label">
          <span>Save what I copy</span>
          <span class="desc">Watch the clipboard, no pasting needed</span>
        </div>
        <input type="checkbox" class="switch" checked={settings.watchClipboard} onchange={(e) => update({ watchClipboard: e.currentTarget.checked })} />
      </label>
    </div>

    <div class="group">
      <label class="item">
        <div class="label">
          <span>Use AI</span>
          <span class="desc">Titles, summaries and smarter links for new items. Optional.</span>
        </div>
        <input type="checkbox" class="switch" checked={settings.ai.enabled} onchange={(e) => updateAi({ enabled: e.currentTarget.checked })} />
      </label>

      {#if settings.ai.enabled}
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
            <input class="input wide" value={settings.ai.baseUrl} placeholder="http://localhost:11434/v1" aria-label="Address" onchange={(e) => updateAi({ baseUrl: e.currentTarget.value.trim() })} />
          </div>
        {/if}

        <div class="item">
          <div class="label"><span>Model</span></div>
          <input class="input wide" value={settings.ai.model} placeholder="model name" aria-label="Model" onchange={(e) => updateAi({ model: e.currentTarget.value.trim() })} />
        </div>

        <label class="item">
          <div class="label">
            <span>Describe images</span>
            <span class="desc">Needs a model that can see pictures</span>
          </div>
          <input type="checkbox" class="switch" checked={settings.ai.vision} onchange={(e) => updateAi({ vision: e.currentTarget.checked })} />
        </label>

        <div class="item" title="The most AI requests per day. 0 means no limit.">
          <div class="label">
            <span>Daily limit</span>
            <span class="desc">AI requests per day, to keep costs predictable</span>
          </div>
          <input class="input num" type="number" min="0" step="50" value={settings.ai.dailyLimit} aria-label="Daily limit" onchange={(e) => updateAi({ dailyLimit: Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)) })} />
        </div>

        <div class="item">
          <div class="label">
            <span class:ok={test.state === 'ok'} class:error={test.state === 'failed'} class="result">
              {test.state === 'running' ? 'Checking…' : test.message || 'Check that it works'}
            </span>
          </div>
          <button class="btn" onclick={runTest} disabled={test.state === 'running' || !settings.ai.model}>Test</button>
        </div>
      {/if}
    </div>

    <div class="group">
      <div class="item">
        <div class="label">
          <span>Library</span>
          <span class="desc path" title={settings.libraryPath}>{settings.libraryPath}</span>
        </div>
      </div>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(460px, calc(100vw - 32px));
    padding: 0 0 8px;
    border: 1px solid var(--border);
    border-radius: 14px;
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.4);
    backdrop-filter: blur(2px);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 4px 20px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 650;
  }

  .group {
    margin: 10px 14px 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg);
  }

  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 52px;
    padding: 8px 14px;
  }

  .item + .item {
    border-top: 1px solid var(--border);
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
    width: 230px;
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
</style>
