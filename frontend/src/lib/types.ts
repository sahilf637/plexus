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
  /** The first-run setup was finished or skipped. */
  setupDone: boolean
}

/** A library folder being considered in setup. */
export interface FolderInfo {
  path: string
  /** Notes it already holds. */
  notes: number
  /** It is the library open now. */
  current: boolean
}

export type ProviderKind = 'openai' | 'anthropic' | 'gemini'

/** The AI side of the settings. Keys are not here: each connection's key lives in the system
 * keychain, under the connection's id. */
export interface AiSettings {
  /** Enrich new items (titles, summaries, concepts, typed links). */
  enabled: boolean
  /** Places requests can go, each with its own key. */
  connections: AiConnection[]
  /** The id of the connection in use. */
  active: string
  /** Most calls per day; 0 means no limit. */
  dailyLimit: number
  /** Most questions asked of the library per day; 0 means no limit. */
  askPerDay: number
  /** How much each question may spend: small for free plans, big for pay-per-use ones. */
  askProfile: 'economy' | 'standard' | 'thorough'
  /** Slowly improve the notes saved before AI was switched on, a few a day. */
  improveOld: boolean
  /** Most older notes improved per day; 0 means only the daily limit applies. */
  improvePerDay: number
}

/** One provider, model and key. */
export interface AiConnection {
  /** Names the keychain entry; never changes. */
  id: string
  /** What the user calls it. */
  name: string
  /** Which preset it was made from. */
  preset: string
  kind: ProviderKind
  baseUrl: string
  model: string
  /** The model can read images, so screenshots and photos get described. */
  vision: boolean
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

/** Why checking a provider or listing its models failed. `message` is a plain sentence, never the
 * provider's own words. `kind` says what to blame: the `key`, the `address`, a `network` problem,
 * the provider being `busy`, or the `model` (anything else). */
export interface AiProblem {
  kind: 'key' | 'address' | 'network' | 'busy' | 'model' | 'other'
  message: string
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
  /** The title was written by the AI (and not changed by hand since). */
  titleByAi: boolean
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

/** Who gave a topic its name: Plexus from the notes' words, the AI, or the user. */
export type LabelBy = 'auto' | 'ai' | 'you'

/** A group of items. Topics form a tree (top-level topics, sub-topics, …). */
export interface Topic {
  id: string
  parentId: string | null
  /** 0 for the top level. */
  level: number
  label: string
  labelBy: LabelBy
  /** One line about the topic, when the AI wrote one. */
  summary: string | null
  /** Items in it, counting those in its sub-topics. */
  count: number
}

/** The topic tree and each item's lowest topic (its ancestors follow the parent chain). */
export interface TopicsData {
  topics: Topic[]
  itemTopic: Record<string, string>
}

/** Sent when an item has joined a topic. */
export interface ItemTopic {
  id: string
  topicId: string
}

/** How the slow pass over older notes is going. */
export interface OlderProgress {
  /** Older notes looked at so far. */
  done: number
  /** All older notes. */
  total: number
  /** Improved today. */
  today: number
}

export interface MapData {
  items: Item[]
  edges: Edge[]
  positions: Position[]
  concepts: ConceptItems[]
  topics: TopicsData
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

/** How a question ended, or that it still is going. `partial`: out of steps, so the answer is the
 * best so far. `failed`: a provider error; it may be resumable while the app stays open. */
export type AskState = 'running' | 'done' | 'partial' | 'stopped' | 'failed'

/** A note an answer rests on: `n` is the number its `[n]` marks in the text refer to. */
export interface AskSource {
  n: number
  itemId: string
  /** A phrase copied from the note, checked to really be there. */
  quote: string
}

/** A question asked of the library, with its answer once there is one. */
export interface Ask {
  id: string
  /** The first question of the conversation this belongs to (its own id when it is the first). */
  threadId: string
  /** A topic or item id the question was limited to. */
  scope: string | null
  question: string
  answer: string
  effort: 'lookup' | 'explain' | 'survey'
  state: AskState
  /** It cited sources and none held up. */
  unsupported: boolean
  /** For a partial answer: what ran out. */
  limitHit: 'steps' | 'tokens' | null
  createdAt: string
  sources: AskSource[]
}

/** One tool the agent ran, sent as it goes. `items` are the notes it touched. */
export interface AskStep {
  askId: string
  n: number
  tool: 'search' | 'read' | 'neighbors' | 'topics'
  /** Plain words for what it did ("searched for \"borrowing\""). */
  label: string
  items: string[]
}

export interface AskFailed {
  askId: string
  message: string
  canResume: boolean
}

/** The result of trying a model on a small made-up question (Settings → AI → Check Ask). */
export interface AskCheck {
  ok: boolean
  /** `tools`: the model calls tools. `json`: it is asked for JSON steps instead. */
  protocol: 'tools' | 'json'
  message: string
}

/** What the AI has been used for, for the usage page (Settings → AI). */
export interface UsageTotals {
  calls: number
  failed: number
  limited: number
  tokensIn: number
  tokensOut: number
}

export interface UsageReport {
  today: UsageTotals
  /** The last two weeks, oldest first, each with what its requests were for. */
  days: { day: string; calls: number; tokens: number; by: { purpose: string; calls: number; tokens: number }[] }[]
  /** Today, by what the calls were for (`notes`, `images`, `topics`, `ask`, `check`, `other`). */
  byPurpose: ({ purpose: string } & UsageTotals)[]
  /** Calls counted against the daily limit today. */
  countedToday: number
  /** Questions asked today. */
  asksToday: number
}
