<script lang="ts">
  // The capture window: paste, drop or type anything, Enter to save, Esc to close. Images and
  // files sit in a tray at the top; the text below is either the thing being saved or, with
  // attachments, a note about them.
  import { onMount } from 'svelte'
  import {
    captureBytes,
    captureFile,
    captureMany,
    capturePreview,
    captureText,
    hideCapture,
    on,
    onFileDrop,
  } from '../lib/api'
  import { isPlainText, resolve, snapshot } from '../lib/paste'
  import { formatBytes, kindLabel } from '../lib/kinds'
  import Icon from '../components/Icon.svelte'
  import type { CaptureResult, Preview } from '../lib/types'

  /** An image or file waiting to be saved. */
  type Attachment = {
    id: number
    name: string
    kind: 'image' | 'file'
    size: number | null
    /** Pasted content; dropped files are sent by path instead. */
    blob: Blob | null
    path: string | null
    thumbnail: string | null
  }

  type Phase = 'edit' | 'saving' | 'secret' | 'saved' | 'error'

  const MAX_ATTACHMENTS = 10
  /** The confirmation shows briefly, then the window closes; linking carries on in the background
   * and shows on the map. */
  const CLOSE_AFTER_SAVE_MS = 600
  const CLOSE_AFTER_DUPLICATE_MS = 1400

  let text = $state('')
  let attachments = $state<Attachment[]>([])
  let nextId = 0
  let preview = $state<Preview | null>(null)
  let phase = $state<Phase>('edit')
  /** The rich-text version of what was pasted into an empty box. */
  let rich: { html: string; plain: string } | null = null

  let secretReason = $state('')
  let savedTitle = $state('')
  /** Nothing new was saved: it was all in Plexus already. */
  let duplicate = $state(false)
  /** Saved alongside, but already in Plexus (when several attachments were saved). */
  let alreadySaved = $state(0)
  let errorMessage = $state('')
  let dragOver = $state(false)
  let textarea = $state<HTMLTextAreaElement>()

  let previewTimer: ReturnType<typeof setTimeout> | undefined
  let closeTimer: ReturnType<typeof setTimeout> | undefined

  const hasNote = $derived(text.trim() !== '')
  const hasContent = $derived(attachments.length > 0 || hasNote)

  const chip = $derived.by(() => {
    if (attachments.length) {
      const images = attachments.filter((a) => a.kind === 'image').length
      const files = attachments.length - images
      const parts = [
        images && (images === 1 ? 'Image' : `${images} images`),
        files && (files === 1 ? 'File' : `${files} files`),
      ].filter(Boolean)
      if (attachments.length === 1) return parts.join('') + (hasNote ? ' · with note' : '')
      return parts.join(' + ') + (hasNote ? ' · one item' : ` · ${attachments.length} items`)
    }
    if (!hasNote || !preview) return ''
    return preview.kind === 'code' && preview.language
      ? `${kindLabel.code} · ${preview.language}`
      : kindLabel[preview.kind]
  })

  function reset() {
    clearTimeout(closeTimer)
    attachments.forEach((a) => a.thumbnail && URL.revokeObjectURL(a.thumbnail))
    attachments = []
    text = ''
    alreadySaved = 0
    preview = null
    phase = 'edit'
    secretReason = ''
    errorMessage = ''
    rich = null
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

  function add(attachment: Omit<Attachment, 'id'>) {
    if (attachments.length >= MAX_ATTACHMENTS) return
    attachments.push({ ...attachment, id: nextId++ })
    if (phase === 'error') phase = 'edit'
    textarea?.focus()
  }

  function attachBlob(blob: Blob, name: string) {
    const isImage = blob.type.startsWith('image/')
    add({
      name: name || (isImage ? `Pasted image.${blob.type.split('/')[1] ?? 'png'}` : 'Pasted file'),
      kind: isImage ? 'image' : 'file',
      size: blob.size,
      blob,
      path: null,
      thumbnail: isImage ? URL.createObjectURL(blob) : null,
    })
  }

  function attachPath(path: string) {
    const name = path.split(/[\\/]/).pop() ?? path
    const isImage = /\.(png|jpe?g|gif|webp|svg)$/i.test(name)
    add({ name, kind: isImage ? 'image' : 'file', size: null, blob: null, path, thumbnail: null })
  }

  function remove(id: number) {
    const attachment = attachments.find((a) => a.id === id)
    if (attachment?.thumbnail) URL.revokeObjectURL(attachment.thumbnail)
    attachments = attachments.filter((a) => a.id !== id)
    textarea?.focus()
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
      // Kept so a paste from a web page or document saves with its headings, lists and links,
      // as long as it isn't edited afterwards (see `save`).
      rich = pasted.html && !text.trim() ? { html: pasted.html, plain: pasted.plain } : null
      // The textarea takes it as usual. If something else had focus, put it there.
      if (document.activeElement !== textarea) {
        event.preventDefault()
        insertText(pasted.plain)
      }
      return
    }
    // The answer may need the backend, so stop the default paste and do it ourselves.
    event.preventDefault()
    for (const content of await resolve(pasted)) {
      if (content.kind === 'blob') attachBlob(content.blob, content.name)
      else if (content.kind === 'path') attachPath(content.path)
      else insertText(content.text)
    }
  }

  // Drops from a browser (dev:ui) arrive as DOM events; in the app, files come through Tauri.
  function ondrop(event: DragEvent) {
    event.preventDefault()
    dragOver = false
    const files = [...(event.dataTransfer?.files ?? [])]
    if (files.length) files.forEach((file) => attachBlob(file, file.name))
    else {
      const dropped = event.dataTransfer?.getData('text/plain')
      if (dropped) insertText(dropped)
    }
  }

  async function save(force = false) {
    if (!hasContent || phase === 'saving') return
    phase = 'saving'
    try {
      const results: CaptureResult[] = []
      if (attachments.length > 1 && hasNote) {
        // A note ties them together: one item holding all of them.
        const parts = attachments.map((a) => (a.path ? { path: a.path } : { blob: a.blob!, name: a.name }))
        const result = await captureMany(parts, text, force)
        if (result.status === 'secret') return askAboutSecret(result.reason)
        results.push(result)
      } else if (attachments.length) {
        // Without a note, each one is its own item.
        for (const a of attachments) {
          const result = a.path
            ? await captureFile(a.path, text, force)
            : await captureBytes(a.blob!, a.name, text, force)
          // A note that looks like a secret stops the first save, before anything is stored.
          if (result.status === 'secret') return askAboutSecret(result.reason)
          results.push(result)
        }
      } else {
        const unedited = rich !== null && text.trim() === rich.plain.trim()
        const result = await captureText(text, force, unedited ? rich!.html : undefined)
        if (result.status === 'secret') return askAboutSecret(result.reason)
        results.push(result)
      }
      showSaved(results)
    } catch (e) {
      errorMessage = String(e)
      phase = 'error'
    }
  }

  function askAboutSecret(reason: string) {
    secretReason = reason
    phase = 'secret'
  }

  function showSaved(results: CaptureResult[]) {
    const saved = results.flatMap((r) => (r.status === 'saved' ? [r] : []))
    const duplicates = results.flatMap((r) => (r.status === 'duplicate' ? [r] : []))
    phase = 'saved'
    duplicate = saved.length === 0
    alreadySaved = duplicates.length
    savedTitle = duplicate
      ? (duplicates[0]?.title ?? '')
      : saved.length === 1
        ? saved[0].title
        : `${saved.length} items saved`
    closeTimer = setTimeout(close, duplicate ? CLOSE_AFTER_DUPLICATE_MS : CLOSE_AFTER_SAVE_MS)
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
    } else if (event.key === 'Backspace' && attachments.length && atStartOfText()) {
      // Like a chat box: backspace at the start of the note removes the last attachment.
      event.preventDefault()
      remove(attachments[attachments.length - 1].id)
    }
  }

  function atStartOfText(): boolean {
    if (document.activeElement !== textarea || !textarea) return true
    return textarea.selectionStart === 0 && textarea.selectionEnd === 0
  }

  onMount(() => {
    reset()
    const unsubscribers = [
      on('capture-opened', reset),
      onFileDrop({
        over: (over) => (dragOver = over),
        drop: (paths) => phase === 'edit' && paths.forEach(attachPath),
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
  {#if phase === 'saved'}
    <div class="done">
      <div class="check" aria-hidden="true"><Icon name="check" size={22} /></div>
      <p class="saved-title">{savedTitle}</p>
      <p class="muted">{duplicate ? 'Already in Plexus' : alreadySaved ? `Saved · ${alreadySaved} already in Plexus` : 'Saved'}</p>
    </div>
  {:else}
    <div class="body">
      {#if attachments.length}
        <ul class="tray" aria-label="Attachments">
          {#each attachments as a (a.id)}
            <li class="tile" title={a.name}>
              {#if a.thumbnail}
                <img src={a.thumbnail} alt={a.name} />
              {:else}
                <div class="file">
                  <span class="ext">{a.name.includes('.') ? a.name.split('.').pop()?.slice(0, 4) : a.kind}</span>
                  <span class="name">{a.name}</span>
                  {#if a.size !== null}<span class="size">{formatBytes(a.size)}</span>{/if}
                </div>
              {/if}
              <button class="remove" onclick={() => remove(a.id)} aria-label={`Remove ${a.name}`}><Icon name="close" size={11} /></button>
            </li>
          {/each}
        </ul>
      {/if}

      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        bind:this={textarea}
        bind:value={text}
        oninput={() => phase === 'secret' && (phase = 'edit')}
        placeholder={attachments.length > 1
          ? 'Add a note to keep these together…'
          : attachments.length
            ? 'Add a note (optional)'
            : 'Paste, drop or type anything'}
        spellcheck="false"
        autofocus
        disabled={phase === 'saving'}
      ></textarea>

      {#if phase === 'secret' || (phase === 'edit' && preview?.secret)}
        <div class="warning" role="alert">
          <span>Looks like {phase === 'secret' ? secretReason : preview?.secret}, so it won't be saved unless you say so.</span>
          {#if phase === 'secret'}
            <button class="btn" onclick={() => save(true)}>Save anyway</button>
          {/if}
        </div>
      {/if}
      {#if phase === 'error'}
        <div class="warning" role="alert"><span>Couldn't save: {errorMessage}</span></div>
      {/if}
    </div>
  {/if}

  {#if phase !== 'saved'}
    <footer>
      <span class="left">
        {#if chip}<span class="chip">{chip}</span>{/if}
        <span class="hints"><kbd>Esc</kbd> close</span>
      </span>
      <button class="btn primary" onclick={() => save(phase === 'secret')} disabled={!hasContent || phase === 'saving'}>
        {phase === 'secret' ? 'Save anyway' : phase === 'saving' ? 'Saving…' : 'Save'}
        {#if phase === 'edit'}<kbd>↵</kbd>{/if}
      </button>
    </footer>
  {/if}
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
    transition: border-color 120ms, box-shadow 120ms;
  }

  .drag-over {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 2px var(--accent-soft);
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 10px 8px 14px;
    border-top: 1px solid var(--border);
  }

  .left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .chip {
    font-size: 12px;
    font-weight: 500;
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    white-space: nowrap;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 16px 10px;
  }

  .tray {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    margin: 0;
    padding: 0 0 4px;
    list-style: none;
    overflow-x: auto;
  }

  .tile {
    position: relative;
    flex-shrink: 0;
    width: 76px;
    height: 76px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface-2);
    overflow: hidden;
  }

  .tile img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .file {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    padding: 6px;
    text-align: center;
  }

  .ext {
    align-self: center;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent-soft);
    color: var(--accent);
    font: 600 10px/1.4 var(--mono);
    text-transform: uppercase;
  }

  .name,
  .size {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }

  .size {
    color: var(--text-muted);
  }

  .remove {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.6);
    color: white;
    opacity: 0;
    transition: opacity 100ms;
  }

  .tile:hover .remove,
  .remove:focus-visible {
    opacity: 1;
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
    font: 15px/1.55 var(--font);
  }

  textarea::placeholder {
    color: var(--text-muted);
  }

  .warning {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 8px 8px 12px;
    border-radius: 8px;
    background: var(--warning-soft);
    color: var(--warning);
    font-size: 13px;
  }

  .warning .btn {
    flex-shrink: 0;
    height: 26px;
    border-color: currentColor;
    background: none;
    color: inherit;
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
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    margin-bottom: 6px;
    animation: pop 220ms ease-out;
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

  .muted,
  .hints {
    color: var(--text-muted);
    font-size: 12px;
  }

  .primary kbd {
    border-color: rgb(255 255 255 / 0.35);
    background: rgb(255 255 255 / 0.15);
    color: white;
  }

  @keyframes pop {
    from {
      transform: scale(0.7);
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .check {
      animation: none;
    }
  }
</style>
