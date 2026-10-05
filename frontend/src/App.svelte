<script lang="ts">
  import { onMount } from 'svelte'
  import { hotkeyStatus, showCapture } from './lib/api'
  import EmptyState from './components/EmptyState.svelte'
  import SettingsDialog from './components/SettingsDialog.svelte'

  let hotkey = $state('Ctrl+Alt+V')
  let settingsOpen = $state(false)

  async function refreshHotkey() {
    const status = await hotkeyStatus()
    if (status.trigger) hotkey = status.trigger
  }

  onMount(() => {
    refreshHotkey()
    // The portal may still be waiting for the user to approve the shortcut at startup.
    const timer = setTimeout(refreshHotkey, 3000)
    return () => clearTimeout(timer)
  })
</script>

<div class="app">
  <header>
    <span class="brand">Plexus</span>
    <div class="actions">
      <button onclick={showCapture}>Add <kbd>{hotkey}</kbd></button>
      <button onclick={() => (settingsOpen = true)} aria-label="Settings">Settings</button>
    </div>
  </header>
  <main>
    <EmptyState {hotkey} />
  </main>
</div>

{#if settingsOpen}
  <SettingsDialog
    onclose={() => {
      settingsOpen = false
      refreshHotkey()
    }}
  />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .brand {
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .actions button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  .actions button:hover {
    border-color: var(--accent);
  }

  main {
    flex: 1;
    min-height: 0;
    position: relative;
  }
</style>
