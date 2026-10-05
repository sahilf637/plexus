<script lang="ts">
  import { onMount } from 'svelte'
  import { getSettings, showCapture } from './lib/api'
  import EmptyState from './components/EmptyState.svelte'

  let hotkey = $state('Ctrl+Alt+V')

  onMount(async () => {
    hotkey = (await getSettings()).hotkey
  })
</script>

<div class="app">
  <header>
    <span class="brand">Plexus</span>
    <button class="add" onclick={showCapture}>Add <kbd>{hotkey}</kbd></button>
  </header>
  <main>
    <EmptyState {hotkey} />
  </main>
</div>

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

  .add {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  .add:hover {
    border-color: var(--accent);
  }

  main {
    flex: 1;
    min-height: 0;
    position: relative;
  }
</style>
