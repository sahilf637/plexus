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
