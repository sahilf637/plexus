<script lang="ts">
  // Placeholder: the real paste/drop/preview flow lands in M0 step 2.
  import { hideCapture } from '../lib/api'

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') hideCapture()
  }
</script>

<svelte:window {onkeydown} />

<div class="capture">
  <p>Paste or drop anything</p>
  <p class="hint"><kbd>Esc</kbd> to close</p>
</div>

<style>
  .capture {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  p {
    margin: 0;
  }

  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }
</style>
