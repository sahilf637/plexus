<script lang="ts">
  // The capture window: paste, drop or type anything, Enter to save, Esc to close.
  import { onMount } from 'svelte'
  import {
    captureBytes,
    captureFile,
    capturePreview,
    captureText,
    hideCapture,
    on,
    onFileDrop,
  } from '../lib/api'
  import { isPlainText, resolve, snapshot } from '../lib/paste'
  import { formatBytes, kindLabel } from '../lib/kinds'
  import type { CaptureResult, ItemLinked, Kind, Preview } from '../lib/types'

  /** An image or file to save instead of text. */
  type Attachment = {
    name: string
    kind: Kind
    size: number | null
    /** Pasted content; dropped files are sent by path instead. */
    blob: Blob | null
    path: string | null
    thumbnail: string | null
  }

  type Phase = 'edit' | 'saving' | 'secret' | 'saved' | 'error'

  const CLOSE_AFTER_SAVE_MS = 1400
  /** How long to wait for links before closing anyway, and how long to show them. */
  const WAIT_FOR_LINKS_MS = 3000
  const SHOW_LINKS_MS = 1800

  let text = $state('')
  let attachment = $state<Attachment | null>(null)
  let preview = $state<Preview | null>(null)
  let phase = $state<Phase>('edit')
  let secretReason = $state('')
  let savedTitle = $state('')
  /** The capture was already in Plexus, so nothing new was saved. */
  let duplicate = $state(false)
  let savedId = $state<string | null>(null)
  /** Titles of what the saved item linked to, once known. */
  let linkedTo = $state<string[] | null>(null)
  let errorMessage = $state('')
  let dragOver = $state(false)
  let textarea = $state<HTMLTextAreaElement>()

  let previewTimer: ReturnType<typeof setTimeout> | undefined
  let closeTimer: ReturnType<typeof setTimeout> | undefined

  const kind = $derived<Kind | null>(attachment?.kind ?? (text.trim() ? (preview?.kind ?? null) : null))
  const chip = $derived.by(() => {
    if (!kind) return ''
    const language = !attachment && kind === 'code' ? preview?.language : null
    return language ? `${kindLabel[kind]} · ${language}` : kindLabel[kind]
  })
  const hasContent = $derived(attachment !== null || text.trim() !== '')

  function reset() {
    clearTimeout(closeTimer)
    if (attachment?.thumbnail) URL.revokeObjectURL(attachment.thumbnail)
    text = ''
    savedId = null
    linkedTo = null
    attachment = null
    preview = null
    phase = 'edit'
    secretReason = ''
    errorMessage = ''
    dragOver = false
    queueMicrotask(() => textarea?.focus())
  }

  function close() {
    reset()
    hideCapture()
  }

  // Ask the backend what the text is (and whether it looks like a secret) as it changes.
  $effect(() => {
    const current = text
    clearTimeout(previewTimer)
    if (!current.trim()) {
      preview = null
      return
    }
    previewTimer = setTimeout(async () => {
      const result = await capturePreview(current)
      if (text === current) preview = result
    }, 120)
  })

  function attachBlob(blob: Blob, name: string) {
    const isImage = blob.type.startsWith('image/')
    attachment = {
      name: name || (isImage ? `Pasted image.${blob.type.split('/')[1] ?? 'png'}` : 'Pasted file'),
      kind: isImage ? 'image' : 'file',
      size: blob.size,
      blob,
      path: null,
      thumbnail: isImage ? URL.createObjectURL(blob) : null,
    }
  }

  function attachPath(path: string) {
    const name = path.split(/[\\/]/).pop() ?? path
    const isImage = /\.(png|jpe?g|gif|webp|svg)$/i.test(name)
    attachment = { name, kind: isImage ? 'image' : 'file', size: null, blob: null, path, thumbnail: null }
  }

  /** Inserts text at the cursor, as a normal paste would have. */
  function insertText(pasted: string) {
    if (!textarea) {
      text += pasted
      return
    }
    textarea.focus()
    textarea.setRangeText(pasted, textarea.selectionStart, textarea.selectionEnd, 'end')
    text = textarea.value
  }

  async function onpaste(event: ClipboardEvent) {
    if (!event.clipboardData || phase !== 'edit') return
    const pasted = snapshot(event.clipboardData)
    if (isPlainText(pasted)) {
      // The textarea takes it as usual. If something else had focus, put it there.
      if (document.activeElement !== textarea) {
        event.preventDefault()
        insertText(pasted.plain)
      }
      return
    }
    // The answer may need the backend, so stop the default paste and do it ourselves.
    event.preventDefault()
    const content = await resolve(pasted)
    if (content?.kind === 'blob') attachBlob(content.blob, content.name)
    else if (content?.kind === 'path') attachPath(content.path)
    else if (content?.kind === 'text') insertText(content.text)
  }

  // Drops from a browser (dev:ui) arrive as DOM events; in the app, files come through Tauri.
  function ondrop(event: DragEvent) {
    event.preventDefault()
    dragOver = false
    const file = event.dataTransfer?.files[0]
    if (file) attachBlob(file, file.name)
    else {
      const dropped = event.dataTransfer?.getData('text/plain')
      if (dropped) text = dropped
    }
  }

  async function save(force = false) {
    if (!hasContent || phase === 'saving') return
    phase = 'saving'
    try {
      let result: CaptureResult
      if (attachment?.path) result = await captureFile(attachment.path)
      else if (attachment?.blob) result = await captureBytes(attachment.blob, attachment.name)
      else result = await captureText(text, force)

      if (result.status === 'secret') {
        secretReason = result.reason
        phase = 'secret'
        return
      }
      savedTitle = result.title
      duplicate = result.status === 'duplicate'
      phase = 'saved'
      if (duplicate) {
        closeTimer = setTimeout(close, CLOSE_AFTER_SAVE_MS)
      } else {
        // Stay open briefly to show what it linked to.
        savedId = result.id
        closeTimer = setTimeout(close, WAIT_FOR_LINKS_MS)
      }
    } catch (e) {
      errorMessage = String(e)
      phase = 'error'
    }
  }

  function removeAttachment() {
    if (attachment?.thumbnail) URL.revokeObjectURL(attachment.thumbnail)
    attachment = null
    textarea?.focus()
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) return
    if (phase === 'saved') {
      close()
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      close()
    } else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      // Keeping a secret takes a deliberate click on "Save anyway", never a second Enter.
      if (phase === 'edit') save()
    } else if (event.key === 'Backspace' && attachment && document.activeElement !== textarea) {
      removeAttachment()
    }
  }

  function onItemLinked(event: ItemLinked) {
    if (phase !== 'saved' || event.id !== savedId) return
    linkedTo = event.related.slice(0, 3).map((r) => r.title)
    clearTimeout(closeTimer)
    closeTimer = setTimeout(close, SHOW_LINKS_MS)
  }

  onMount(() => {
    reset()
    const unsubscribers = [
      on('capture-opened', reset),
      on<ItemLinked>('item-linked', onItemLinked),
      onFileDrop({
        over: (over) => (dragOver = over),
        drop: (paths) => paths[0] && phase === 'edit' && attachPath(paths[0]),
      }),
    ]
    return () => unsubscribers.forEach((u) => u.then((off) => off()))
  })
</script>

<svelte:window {onkeydown} {onpaste} />

<div
  class="capture"
  class:drag-over={dragOver}
  role="application"
  ondragover={(e) => {
    e.preventDefault()
    dragOver = true
  }}
  ondragleave={() => (dragOver = false)}
  {ondrop}
>
  <header>
    <span class="title">Add to Plexus</span>
    {#if chip && phase === 'edit'}<span class="chip">{chip}</span>{/if}
  </header>

  {#if phase === 'saved'}
    <div class="done">
      <div class="check" aria-hidden="true">✓</div>
      <p class="saved-title">{savedTitle}</p>
      {#if duplicate}
        <p class="muted">Already in Plexus, so nothing new was saved.</p>
      {:else if linkedTo === null}
        <p class="muted">Saved. Finding related items…</p>
      {:else if linkedTo.length}
        <p class="muted">Linked to</p>
        <ul class="linked">
          {#each linkedTo as title}<li>{title}</li>{/each}
        </ul>
      {:else}
        <p class="muted">Saved. Nothing related yet; it will link up as you add more.</p>
      {/if}
    </div>
  {:else}
    <div class="body">
      {#if attachment}
        <div class="attachment">
          {#if attachment.thumbnail}
            <img src={attachment.thumbnail} alt="" />
          {:else}
            <div class="file-icon" aria-hidden="true">{attachment.kind === 'image' ? '🖼' : '📄'}</div>
          {/if}
          <div class="file-info">
            <span class="file-name">{attachment.name}</span>
            {#if attachment.size !== null}<span class="muted">{formatBytes(attachment.size)}</span>{/if}
          </div>
          <button class="remove" onclick={removeAttachment} aria-label="Remove">✕</button>
        </div>
      {:else}
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          bind:this={textarea}
          bind:value={text}
          oninput={() => phase === 'secret' && (phase = 'edit')}
          placeholder="Paste or drop anything, or type…"
          spellcheck="false"
          autofocus
          disabled={phase === 'saving'}
        ></textarea>
      {/if}

      {#if phase === 'secret' || (phase === 'edit' && preview?.secret && !attachment)}
        <div class="warning" role="alert">
          <span>
            This looks like {phase === 'secret' ? secretReason : preview?.secret}. It stays out of
            Plexus unless you choose to keep it.
          </span>
          {#if phase === 'secret'}
            <button onclick={() => save(true)}>Save anyway</button>
          {/if}
        </div>
      {/if}
      {#if phase === 'error'}
        <div class="warning" role="alert"><span>Couldn't save: {errorMessage}</span></div>
      {/if}
    </div>
  {/if}

  <footer>
    {#if phase === 'saved'}
      <span class="muted">Press any key to close</span>
    {:else}
      <span class="hints">
        <kbd>Enter</kbd> save · <kbd>Shift</kbd>+<kbd>Enter</kbd> new line · <kbd>Esc</kbd> close
      </span>
      <button class="primary" onclick={() => save(phase === 'secret')} disabled={!hasContent || phase === 'saving'}>
        {phase === 'secret' ? 'Save anyway' : 'Save'}
      </button>
    {/if}
  </footer>
</div>

<style>
  .capture {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    transition: border-color 120ms;
  }

  .drag-over {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 2px var(--accent-soft);
  }

  header,
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
  }

  header {
    border-bottom: 1px solid var(--border);
  }

  .title {
    font-weight: 600;
  }

  .chip {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px 0;
  }

  textarea {
    flex: 1;
    min-height: 0;
    width: 100%;
    resize: none;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font: 14px/1.55 var(--font);
  }

  textarea::placeholder {
    color: var(--text-muted);
  }

  .attachment {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .attachment img {
    width: 96px;
    height: 72px;
    object-fit: cover;
    border-radius: 6px;
  }

  .file-icon {
    width: 48px;
    text-align: center;
    font-size: 28px;
  }

  .file-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .remove {
    border: none;
    background: none;
    color: var(--text-muted);
    padding: 4px 8px;
  }

  .warning {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--warning-soft);
    color: var(--warning);
    font-size: 13px;
  }

  .warning button {
    flex-shrink: 0;
    border: 1px solid currentColor;
    border-radius: 6px;
    background: none;
    padding: 3px 8px;
  }

  .done {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0 24px;
    text-align: center;
  }

  .check {
    width: 40px;
    height: 40px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 20px;
    margin-bottom: 6px;
  }

  .saved-title {
    margin: 0;
    font-weight: 600;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .done p {
    margin: 0;
  }

  .linked {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: 100%;
  }

  .linked li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted,
  .hints {
    color: var(--text-muted);
    font-size: 12px;
  }

  .primary {
    border: none;
    border-radius: 8px;
    padding: 6px 14px;
    background: var(--accent);
    color: white;
    font-weight: 500;
  }

  .primary:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
