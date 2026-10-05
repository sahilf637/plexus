<script lang="ts">
  let { hotkey }: { hotkey: string } = $props()
</script>

<div class="empty">
  <svg viewBox="0 0 120 80" aria-hidden="true">
    <g stroke="currentColor" stroke-width="1.5" opacity="0.35">
      <line x1="20" y1="50" x2="52" y2="22" />
      <line x1="52" y1="22" x2="92" y2="34" />
      <line x1="52" y1="22" x2="66" y2="62" />
      <line x1="92" y1="34" x2="66" y2="62" />
    </g>
    <g fill="currentColor">
      <circle cx="20" cy="50" r="5" opacity="0.5" />
      <circle cx="52" cy="22" r="7" />
      <circle cx="92" cy="34" r="5" opacity="0.7" />
      <circle cx="66" cy="62" r="6" opacity="0.6" />
    </g>
  </svg>
  <h1>Your map is empty</h1>
  <p>Press <kbd>{hotkey}</kbd> anywhere, paste anything, and press <kbd>Enter</kbd>.</p>
  <p class="muted">Text, links, code, screenshots and files all work. Plexus links them for you.</p>
</div>

<style>
  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    text-align: center;
  }

  svg {
    width: 120px;
    color: var(--accent);
    margin-bottom: 12px;
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  .muted {
    color: var(--text-muted);
  }
</style>
