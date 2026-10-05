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

export type ItemStatus = 'pending' | 'linked' | 'unlinkable' | 'failed'

/** An item in the index (one note file). */
export interface Item {
  id: string
  /** Note file, relative to the library folder. */
  path: string
  kind: Kind
  title: string
  sourceUrl: string | null
  language: string | null
  capturedAt: string
  bodyHash: string
  assetPath: string | null
  assetHash: string | null
  mime: string | null
  size: number | null
  status: ItemStatus
}

export interface ItemDetail extends Item {
  body: string
  /** Absolute path of the original file, if any. */
  assetFile: string | null
}

export interface Edge {
  src: string
  dst: string
  /** "similar" for links found by meaning; typed links come with AI enrichment. */
  kind: string
  weight: number
  reason: string | null
}

export interface Position {
  id: string
  x: number
  y: number
}

export interface MapData {
  items: Item[]
  edges: Edge[]
  positions: Position[]
}

/** Sent when the background pipeline has linked an item. */
export interface ItemLinked {
  id: string
  status: ItemStatus
  /** Every link touching this item, now. */
  edges: Edge[]
  /** Linked items, closest first. */
  related: { id: string; title: string; weight: number }[]
}
