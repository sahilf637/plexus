// Working out what a paste carries. What arrives depends on the source app and the OS: the
// webview hands over text and some files, but on Linux images and files copied in a file
// manager need the backend's help.

import { clipboardImage, pastedFiles } from './api'

export type Pasted =
  | { kind: 'text'; text: string }
  | { kind: 'blob'; blob: Blob; name: string }
  | { kind: 'path'; path: string }

/** Everything we need from a paste event, read synchronously: the clipboard data is gone once
 * the event handler returns. */
export interface PasteSnapshot {
  file: File | null
  plain: string
  uriList: string
  hasImageType: boolean
}

export function snapshot(data: DataTransfer): PasteSnapshot {
  return {
    file: [...data.items].find((item) => item.kind === 'file')?.getAsFile() ?? null,
    plain: data.getData('text/plain'),
    uriList: data.getData('text/uri-list'),
    hasImageType: data.types.some((type) => type.startsWith('image/')),
  }
}

const mayBeFiles = (s: PasteSnapshot) => /^\s*(\/|file:\/\/)/.test(s.uriList || s.plain)
const mayBeImage = (s: PasteSnapshot) => !s.plain.trim() || s.hasImageType

/** Ordinary text the browser can paste by itself. */
export function isPlainText(s: PasteSnapshot): boolean {
  return !s.file && !mayBeFiles(s) && !mayBeImage(s)
}

/** Resolves what was pasted, asking the backend about files and images when needed. */
export async function resolve(s: PasteSnapshot): Promise<Pasted | null> {
  if (s.file) {
    const isImage = s.file.type.startsWith('image/')
    return { kind: 'blob', blob: s.file, name: s.file.name || (isImage ? 'Pasted image.png' : 'Pasted file') }
  }
  if (mayBeFiles(s)) {
    const paths = await pastedFiles(s.uriList || s.plain)
    if (paths.length) return { kind: 'path', path: paths[0] }
  }
  if (mayBeImage(s)) {
    const image = await clipboardImage().catch(() => null)
    if (image) return { kind: 'blob', blob: image, name: 'Pasted image.png' }
  }
  return s.plain.trim() ? { kind: 'text', text: s.plain } : null
}
