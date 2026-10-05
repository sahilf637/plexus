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
  /** Download the web page behind a saved link so it can be searched and linked. */
  fetchLinks: boolean
  /** Save text copied anywhere, without pasting it into Plexus. */
  watchClipboard: boolean
  ai: AiSettings
}

export type ProviderKind = 'openai' | 'anthropic' | 'gemini'

/** The connected AI provider. Its key is not here: it lives in the system keychain. */
export interface AiSettings {
  /** Enrich new items (titles, summaries, concepts, typed links). */
  enabled: boolean
  /** Which preset was picked (also names the keychain entry). */
  preset: string
  kind: ProviderKind
  baseUrl: string
  model: string
  /** The model can read images, so screenshots and photos get described. */
  vision: boolean
  /** Most calls per day; 0 means no limit. */
  dailyLimit: number
}

export interface AiKeyStatus {
  hasKey: boolean
  /** Set when the system keychain couldn't be read. */
  error: string | null
}

/** A model a provider offers. */
export interface ModelInfo {
  id: string
  /** Reads images. */
  vision: boolean
}

/** Where AI enrichment stands, shown in the map's header. */
export interface EnrichStatus {
  state: 'off' | 'idle' | 'working' | 'paused'
  /** Items waiting, including the one being read. */
  pending: number
  /** Why it is paused. */
  message: string | null
}

/** A shared idea that items are tagged with. */
export interface Concept {
  id: number
  name: string
}

export interface ConceptItems extends Concept {
  items: string[]
}

/** Sent when an item's concepts changed. */
export interface ItemConcepts {
  id: string
  concepts: Concept[]
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
  /** What produced the body: "web@1" once a link's page was read, "web-unreadable@1" if it
   * couldn't be. Absent while a link's page hasn't been read yet. */
  extracted: string | null
  /** One line about the item, written by the AI provider. */
  summary: string | null
}

export interface ItemDetail extends Item {
  body: string
  /** Absolute path of the original file, if any. */
  assetFile: string | null
  /** More originals saved with it, after the first. */
  attachments: { name: string; file: string; kind: Kind; size: number | null }[]
  /** Text read out of the original (a PDF's pages, an image's words), apart from the note. */
  extractedText?: string | null
  /** The title was written by the AI, and can go back to `titleWas`. */
  titleByAi: boolean
  titleWas: string | null
}

/** How sure a link is. */
export type Tier = 'strong' | 'related' | 'maybe'

export interface Edge {
  src: string
  dst: string
  /** "similar" for links found by meaning, "user" for hand-made ones, or a type the AI gave
   * (supports, contradicts, extends, example_of, related). For typed links `src` is the item the
   * statement is about: "src extends dst". */
  kind: string
  weight: number
  tier: Tier
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
  concepts: ConceptItems[]
}

/** Items moved to the trash; `batch` restores them. */
export interface Deleted {
  batch: string
  ids: string[]
}

/** Sent when a link made by hand is added or removed: every link touching these items, now. */
export interface LinksChanged {
  ids: string[]
  edges: Edge[]
}

/** Sent when the background pipeline has linked an item. */
export interface ItemLinked {
  id: string
  status: ItemStatus
  /** Every link touching this item, now. */
  edges: Edge[]
  /** Linked items, closest first. */
  related: { id: string; title: string; weight: number; tier: Tier }[]
}
