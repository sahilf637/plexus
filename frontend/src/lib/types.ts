// Shapes shared with the Rust side. Field names match the serde (camelCase) output.

export type HotkeyAction = 'openCapture' | 'saveClipboard'

export interface Settings {
  /** Folder that holds the note files. */
  libraryPath: string
  /** Global shortcut, e.g. "Ctrl+Alt+V". */
  hotkey: string
  /** What the global shortcut does. */
  hotkeyAction: HotkeyAction
  /** Launch at login so the hotkey is always ready. */
  startAtLogin: boolean
}

export interface HotkeyStatus {
  backend: 'portal' | 'native' | 'none'
  /** The keys actually bound, as the system describes them. */
  trigger: string | null
  error: string | null
  /** The system has its own dialog for changing the keys (Wayland portal). */
  systemConfigurable: boolean
}

export type Kind = 'text' | 'code' | 'url' | 'image' | 'file'

export interface Preview {
  kind: Kind
  language: string | null
  title: string
  /** e.g. "a GitHub token" when the text looks like a secret. */
  secret: string | null
}

export type CaptureResult =
  | { status: 'saved'; id: string; title: string; kind: Kind }
  | { status: 'duplicate'; id: string; title: string }
  | { status: 'secret'; reason: string }
