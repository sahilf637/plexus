<script lang="ts">
  import { onMount } from 'svelte'
  import { configureHotkey, getSettings, hotkeyStatus, setSettings } from '../lib/api'
  import type { HotkeyStatus, Settings } from '../lib/types'

  let { onclose }: { onclose: () => void } = $props()

  let settings = $state<Settings | null>(null)
  let status = $state<HotkeyStatus | null>(null)
  let hotkeyDraft = $state('')
  let dialog = $state<HTMLDialogElement>()

  onMount(async () => {
    dialog?.showModal()
    ;[settings, status] = await Promise.all([getSettings(), hotkeyStatus()])
    hotkeyDraft = settings.hotkey
  })

  async function update(patch: Partial<Settings>) {
    if (!settings) return
    settings = await setSettings({ ...settings, ...patch })
    // Registration is asynchronous on some systems; give it a moment before re-reading.
    setTimeout(async () => (status = await hotkeyStatus()), 300)
  }
</script>

<dialog bind:this={dialog} {onclose} onclick={(e) => e.target === dialog && dialog?.close()}>
  <h2>Settings</h2>

  {#if settings && status}
    <section>
      <h3>Shortcut</h3>
      <p class="muted">Opens the capture window from any app, while Plexus is running.</p>
      {#if status.error}
        <p class="error">{status.error}</p>
      {:else}
        <p><kbd>{status.trigger}</kbd>
          {#if status.backend === 'portal'}<span class="muted">· set in your desktop's shortcuts</span>{/if}
        </p>
      {/if}

      {#if status.backend === 'portal' || (status.backend === 'none' && status.systemConfigurable)}
        {#if status.systemConfigurable}
          <button onclick={async () => { await configureHotkey(); setTimeout(async () => (status = await hotkeyStatus()), 1000) }}>
            Change shortcut…
          </button>
        {:else}
          <p class="muted">Change it in your desktop's keyboard settings.</p>
        {/if}
      {:else}
        <div class="row">
          <input bind:value={hotkeyDraft} aria-label="Shortcut" placeholder="Ctrl+Alt+V" />
          <button onclick={() => update({ hotkey: hotkeyDraft.trim() })} disabled={hotkeyDraft.trim() === settings.hotkey}>
            Save
          </button>
        </div>
      {/if}
      <p class="muted small">
        Can't use a global shortcut? Add a custom shortcut in your system settings that runs
        <code>plexus --capture</code>.
      </p>
    </section>

    <section>
      <h3>When the shortcut is pressed</h3>
      <label class="row">
        <input type="radio" name="hotkey-action" checked={settings.hotkeyAction === 'openCapture'} onchange={() => update({ hotkeyAction: 'openCapture' })} />
        Open the capture window, to paste, drop or type
      </label>
      <label class="row">
        <input type="radio" name="hotkey-action" checked={settings.hotkeyAction === 'saveClipboard'} onchange={() => update({ hotkeyAction: 'saveClipboard' })} />
        Save the clipboard straight away, with a notification
      </label>
    </section>

    <section>
      <h3>Start at login</h3>
      <label class="row">
        <input type="checkbox" checked={settings.startAtLogin} onchange={(e) => update({ startAtLogin: e.currentTarget.checked })} />
        Keep Plexus running in the tray so the shortcut always works
      </label>
    </section>

    <section>
      <h3>Library folder</h3>
      <p><code>{settings.libraryPath}</code></p>
      <p class="muted small">Every item is a markdown file here. Changing the folder comes later.</p>
    </section>
  {/if}

  <footer>
    <button onclick={() => dialog?.close()}>Done</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(480px, calc(100vw - 32px));
    padding: 20px 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.35);
  }

  h2 {
    margin: 0 0 12px;
    font-size: 17px;
  }

  h3 {
    margin: 0 0 2px;
    font-size: 14px;
  }

  section {
    padding: 12px 0;
    border-top: 1px solid var(--border);
  }

  p {
    margin: 4px 0;
  }

  .muted {
    color: var(--text-muted);
  }

  .small {
    font-size: 12px;
  }

  .error {
    color: var(--danger);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  input:not([type]) {
    flex: 1;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
  }

  button {
    padding: 5px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  button:disabled {
    opacity: 0.5;
  }

  code {
    font: 12px var(--mono);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    padding-top: 8px;
  }
</style>
