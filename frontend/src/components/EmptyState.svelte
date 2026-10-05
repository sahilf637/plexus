<script lang="ts">
  // What a new library shows instead of the map: one way in, and a hint at the other.
  import Icon from './Icon.svelte'
  let { hotkey, onadd }: { hotkey: string; onadd: () => void } = $props()
</script>

<div class="empty">
  <div class="card">
    <svg viewBox="0 0 160 100" aria-hidden="true">
      <g class="edges" stroke="currentColor" stroke-width="1.5">
        <line x1="28" y1="62" x2="70" y2="28" />
        <line x1="70" y1="28" x2="122" y2="40" />
        <line x1="70" y1="28" x2="88" y2="78" />
        <line x1="122" y1="40" x2="88" y2="78" />
        <line x1="122" y1="40" x2="142" y2="76" stroke-dasharray="3 4" />
      </g>
      <g fill="currentColor">
        <circle cx="28" cy="62" r="6" opacity="0.5" />
        <circle class="pulse" cx="70" cy="28" r="9" />
        <circle cx="122" cy="40" r="6.5" opacity="0.75" />
        <circle cx="88" cy="78" r="7" opacity="0.6" />
        <circle cx="142" cy="76" r="5" opacity="0.3" />
      </g>
    </svg>

    <h1>Your map starts here</h1>
    <button class="btn primary" onclick={onadd}><Icon name="plus" size={15} />Add something <kbd>{hotkey}</kbd></button>
    <p class="hint">or just paste here <kbd>Ctrl</kbd> <kbd>V</kbd></p>
  </div>
</div>

<style>
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 24px;
    overflow-y: auto;
  }

  .card {
    max-width: 480px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    text-align: center;
  }

  svg {
    width: 168px;
    color: var(--accent);
  }

  .edges {
    opacity: 0.35;
  }

  .pulse {
    transform-origin: 70px 28px;
    animation: pulse 2.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      transform: scale(1.18);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }

  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }



  .primary {
    height: 38px;
    margin-top: 4px;
    padding: 0 16px;
    border-radius: 10px;
    box-shadow: 0 4px 14px color-mix(in srgb, var(--accent) 35%, transparent);
  }

  .primary kbd {
    border-color: rgb(255 255 255 / 0.35);
    background: rgb(255 255 255 / 0.15);
    color: white;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }

</style>
