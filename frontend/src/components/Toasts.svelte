<script lang="ts">
  import { dismiss, toasts } from '../lib/toasts.svelte'
</script>

<div class="toasts" aria-live="polite">
  {#each toasts as t (t.id)}
    <div class="toast">
      <span>{t.message}</span>
      {#if t.action}
        <button
          onclick={() => {
            t.action?.run()
            dismiss(t.id)
          }}>{t.action.label}</button
        >
      {/if}
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: absolute;
    left: 16px;
    bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: calc(100% - 32px);
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 10px 9px 14px;
    border: 1px solid var(--border);
    border-radius: 10px;
    animation: rise 160ms ease-out;
    background: var(--surface);
    box-shadow: var(--shadow);
    pointer-events: auto;
  }

  button {
    border: none;
    border-radius: 6px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
    padding: 3px 10px;
  }

  button:hover {
    filter: brightness(0.95);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: none;
    }
  }
</style>
