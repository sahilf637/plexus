<script lang="ts">
  // What a new library shows instead of the map: what Plexus does, and the two ways to start.
  import { onMount } from 'svelte'
  import { getSettings } from '../lib/api'

  let { hotkey, onadd }: { hotkey: string; onadd: () => void } = $props()

  let library = $state('')
  onMount(async () => {
    const path = (await getSettings()).libraryPath
    library = path.replace(/^\/(home|Users)\/[^/]+/, '~')
  })

  const kinds = [
    ['text', 'Notes'],
    ['url', 'Links'],
    ['code', 'Code'],
    ['image', 'Screenshots'],
    ['file', 'Files'],
  ]
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

    <h1>Start your map</h1>
    <p class="lead">
      Paste anything you want to keep. Plexus saves it instantly, then finds what it's related to and
      links it for you.
    </p>

    <div class="actions">
      <button class="primary" onclick={onadd}>Add something <kbd>{hotkey}</kbd></button>
      <span class="or">or press <kbd>Ctrl</kbd>+<kbd>V</kbd> right here</span>
    </div>

    <ul class="kinds" aria-label="What you can add">
      {#each kinds as [kind, label]}
        <li><span class="dot" style:background={`var(--kind-${kind})`}></span>{label}</li>
      {/each}
    </ul>

    <p class="hint">
      <kbd>{hotkey}</kbd> works from any app, even with this window closed.
      {#if library}<br />Everything is kept as plain files in <code>{library}</code>.{/if}
    </p>
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

  .lead {
    margin: 0;
    color: var(--text-muted);
    font-size: 15px;
    line-height: 1.55;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 10px 14px;
    margin-top: 4px;
  }

  .primary {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 16px;
    border: none;
    border-radius: 10px;
    background: var(--accent);
    color: white;
    font-weight: 500;
  }

  .primary:hover {
    filter: brightness(1.08);
  }

  .primary kbd {
    border-color: rgb(255 255 255 / 0.35);
    background: rgb(255 255 255 / 0.15);
    color: white;
  }

  .or {
    color: var(--text-muted);
    font-size: 13px;
  }

  .kinds {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
  }

  .kinds li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    font-size: 12px;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }

  .hint {
    margin: 4px 0 0;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.7;
  }

  code {
    font: 12px var(--mono);
  }
</style>
