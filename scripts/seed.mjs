// Writes a library of a few hundred varied notes for testing and tuning: Wikipedia summaries
// and intros across ~15 topics, code snippets, short personal notes and bare links (which the
// app then reads). Each note carries `seed_topic:` in its frontmatter so link quality can be
// measured (`cargo test tune_links -- --ignored --nocapture`).
//
//   npm run seed -- <folder>                   then: PLEXUS_LIBRARY=<folder> npm run dev
//
// Wikipedia responses are cached in <folder>/.seed-cache.json, so re-running is offline.

import { createHash, randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const target = process.argv[2]
if (!target) {
  console.error('usage: npm run seed -- <folder>')
  process.exit(1)
}
const root = resolve(target)
const itemsDir = join(root, 'items')
if (existsSync(itemsDir)) {
  console.error(`${root} already has items. Delete its items/ and .plexus/ folders to re-seed (the download cache is kept), or pick another folder.`)
  process.exit(1)
}

/** topic → Wikipedia articles */
const articles = {
  rust: ['Rust (programming language)', 'Borrow checker', 'Memory safety', 'Cargo (package manager)', 'Ownership (computer science)', 'Smart pointer', 'Zero-cost abstraction', 'WebAssembly', 'Garbage collection (computer science)', 'Reference counting', 'Data race', 'Type inference'],
  baking: ['Sourdough', 'Bread', 'Baker\'s yeast', 'Gluten', 'Proofing (baking technique)', 'Baguette', 'Lactobacillus sanfranciscensis', 'Dutch oven', 'Maillard reaction', 'Croissant', 'Rye bread', 'Baking stone'],
  memory: ['Spaced repetition', 'Forgetting curve', 'Testing effect', 'Leitner system', 'Mnemonic', 'Method of loci', 'Working memory', 'Long-term memory', 'Hermann Ebbinghaus', 'Interleaved practice', 'Memory consolidation', 'Anki (software)'],
  astronomy: ['Black hole', 'Neutron star', 'Exoplanet', 'James Webb Space Telescope', 'Andromeda Galaxy', 'Dark matter', 'Supernova', 'Red giant', 'Cosmic microwave background', 'Kuiper belt', 'Hubble\'s law', 'Pulsar'],
  climate: ['Climate change', 'Greenhouse effect', 'Carbon dioxide in Earth\'s atmosphere', 'Paris Agreement', 'Sea level rise', 'Carbon capture and storage', 'Ocean acidification', 'Permafrost', 'Renewable energy', 'Heat pump', 'El Niño–Southern Oscillation', 'Carbon tax'],
  jazz: ['Jazz', 'Bebop', 'Miles Davis', 'John Coltrane', 'Kind of Blue', 'Swing music', 'Improvisation', 'Thelonious Monk', 'Modal jazz', 'Blue note', 'Duke Ellington', 'Charlie Parker'],
  chess: ['Chess', 'Sicilian Defence', 'Queen\'s Gambit', 'Magnus Carlsen', 'Stockfish (chess)', 'Chess opening', 'Endgame tablebase', 'Elo rating system', 'Castling', 'En passant', 'Garry Kasparov', 'Chess endgame'],
  ml: ['Machine learning', 'Transformer (deep learning architecture)', 'Backpropagation', 'Gradient descent', 'Convolutional neural network', 'Overfitting', 'Word embedding', 'Large language model', 'Attention (machine learning)', 'Reinforcement learning', 'Random forest', 'Support vector machine'],
  rome: ['Ancient Rome', 'Julius Caesar', 'Roman Republic', 'Augustus', 'Roman legion', 'Colosseum', 'Punic Wars', 'Hadrian\'s Wall', 'Roman roads', 'Fall of the Western Roman Empire', 'Cicero', 'Roman Senate'],
  gardening: ['Gardening', 'Compost', 'Companion planting', 'Tomato', 'Raised-bed gardening', 'Mulch', 'Crop rotation', 'Permaculture', 'Soil pH', 'Pruning', 'Seed saving', 'Vermicompost'],
  coffee: ['Coffee', 'Espresso', 'Coffea arabica', 'Coffee roasting', 'Pour-over coffee', 'AeroPress', 'Cold brew coffee', 'Caffeine', 'French press', 'Latte art', 'Coffee bean', 'Moka pot'],
  running: ['Running', 'Marathon', 'Interval training', 'VO2 max', 'Eliud Kipchoge', 'Lactate threshold', 'Barefoot running', 'Ultramarathon', 'Fartlek', 'Shin splints', 'Running economy', 'Tempo run'],
  photography: ['Photography', 'Aperture', 'Shutter speed', 'Depth of field', 'Rule of thirds', 'Film speed', 'Exposure (photography)', 'Bokeh', 'Golden hour (photography)', 'Mirrorless camera', 'Raw image format', 'Long-exposure photography'],
  economics: ['Inflation', 'Central bank', 'Interest rate', 'Quantitative easing', 'Supply and demand', 'Gross domestic product', 'Recession', 'Stagflation', 'Federal Reserve', 'Phillips curve', 'Monetary policy', 'Unemployment'],
  geology: ['Volcano', 'Plate tectonics', 'Earthquake', 'Mount St. Helens', 'Magma', 'Igneous rock', 'Tsunami', 'Ring of Fire', 'Geyser', 'Lava', 'Subduction', 'Richter scale'],
}

/** [topic, language, code] */
const code = [
  ['rust', 'rust', 'fn longest<\'a>(x: &\'a str, y: &\'a str) -> &\'a str {\n    if x.len() > y.len() { x } else { y }\n}'],
  ['rust', 'rust', 'use std::sync::{Arc, Mutex};\nuse std::thread;\n\nlet counter = Arc::new(Mutex::new(0));\nlet handles: Vec<_> = (0..10).map(|_| {\n    let c = Arc::clone(&counter);\n    thread::spawn(move || *c.lock().unwrap() += 1)\n}).collect();\nfor h in handles { h.join().unwrap(); }'],
  ['rust', 'rust', 'impl Drop for Connection {\n    fn drop(&mut self) {\n        self.close();\n        println!("connection closed");\n    }\n}'],
  ['rust', 'rust', '#[derive(Debug, Clone)]\nstruct Point { x: f64, y: f64 }\n\nfn main() {\n    let p = Point { x: 1.0, y: 2.0 };\n    let q = p.clone(); // p is still usable\n    println!("{p:?} {q:?}");\n}'],
  ['ml', 'python', 'import torch\nimport torch.nn as nn\n\nclass MLP(nn.Module):\n    def __init__(self):\n        super().__init__()\n        self.net = nn.Sequential(nn.Linear(784, 256), nn.ReLU(), nn.Linear(256, 10))\n\n    def forward(self, x):\n        return self.net(x)'],
  ['ml', 'python', 'from sklearn.ensemble import RandomForestClassifier\nfrom sklearn.model_selection import train_test_split\n\nX_train, X_test, y_train, y_test = train_test_split(X, y, test_size=0.2)\nclf = RandomForestClassifier(n_estimators=200).fit(X_train, y_train)\nprint(clf.score(X_test, y_test))'],
  ['ml', 'python', 'def gradient_descent(w, grad, lr=0.01, steps=1000):\n    for _ in range(steps):\n        w = w - lr * grad(w)\n    return w'],
  ['ml', 'python', 'scores = q @ k.transpose(-2, -1) / math.sqrt(d_k)\nweights = scores.softmax(dim=-1)\nout = weights @ v  # scaled dot-product attention'],
  ['economics', 'python', '# Real interest rate (Fisher equation)\ndef real_rate(nominal, inflation):\n    return (1 + nominal) / (1 + inflation) - 1\n\nprint(real_rate(0.05, 0.03))'],
  ['running', 'python', 'def pace(distance_km, minutes):\n    """Minutes per km, e.g. 42.195 km in 180 min -> 4:16/km"""\n    p = minutes / distance_km\n    return f"{int(p)}:{round((p % 1) * 60):02d}/km"'],
  ['photography', 'python', 'from PIL import Image\n\nimg = Image.open("IMG_2041.jpg")\nexif = img.getexif()\nprint("aperture", exif.get(33437), "shutter", exif.get(33434), "ISO", exif.get(34855))'],
  ['astronomy', 'python', 'from astropy import units as u\nfrom astropy.cosmology import Planck18\n\nd = Planck18.luminosity_distance(1.0)\nprint(d.to(u.Gpc))  # distance to a galaxy at redshift z = 1'],
  ['chess', 'python', 'import chess\nimport chess.engine\n\nboard = chess.Board()\nengine = chess.engine.SimpleEngine.popen_uci("stockfish")\ninfo = engine.analyse(board, chess.engine.Limit(depth=20))\nprint(info["score"])'],
  ['memory', 'python', 'def next_interval(interval, ease, quality):\n    """SM-2 spaced repetition: grow the gap between reviews when recall is good."""\n    if quality < 3:\n        return 1, ease\n    ease = max(1.3, ease + 0.1 - (5 - quality) * (0.08 + (5 - quality) * 0.02))\n    return round(interval * ease), ease'],
  ['climate', 'python', 'import pandas as pd\n\nco2 = pd.read_csv("mauna_loa_co2.csv", parse_dates=["date"])\nyearly = co2.groupby(co2.date.dt.year).ppm.mean()\nprint(yearly.diff().tail(10))  # annual CO2 increase in ppm'],
  ['geology', 'python', 'import math\n\ndef magnitude_energy_ratio(m1, m2):\n    """Each whole step on the moment magnitude scale releases ~31.6x more energy."""\n    return 10 ** (1.5 * (m2 - m1))'],
  ['coffee', 'python', 'def brew_ratio(coffee_g, ratio=16):\n    """Water for a pour-over at 1:ratio, e.g. 20 g coffee -> 320 g water."""\n    return coffee_g * ratio'],
  ['baking', 'python', 'def bakers_percent(flour, water, salt, starter):\n    return {\n        "hydration": water / flour * 100,\n        "salt": salt / flour * 100,\n        "starter": starter / flour * 100,\n    }'],
  ['gardening', 'sql', 'SELECT bed, crop, planted_on\nFROM plantings\nWHERE season = \'spring\' AND family = \'nightshade\'\nORDER BY bed;  -- check crop rotation before planting tomatoes'],
  ['jazz', 'python', '# ii-V-I in every key\nkeys = ["C", "F", "Bb", "Eb", "Ab", "Db", "Gb", "B", "E", "A", "D", "G"]\nfor k in keys:\n    print(f"{k}: ii-7  V7  Imaj7")'],
]

/** [topic, note] */
const notes = [
  ['baking', 'Next bake: try 78% hydration and a longer cold retard overnight in the fridge. Last loaf spread too much.'],
  ['baking', 'Starter peaks about 6 hours after feeding when the kitchen is warm. Use it at peak for the best rise.'],
  ['rust', 'Remember: you can have many shared references or one mutable reference, never both at once.'],
  ['rust', 'Lifetimes don\'t change how long values live; they only describe relationships the compiler checks.'],
  ['memory', 'Flashcards work better when I write them myself and keep one fact per card.'],
  ['memory', 'Testing myself the next morning beat rereading the chapter twice. The forgetting curve is real.'],
  ['running', 'Easy runs should feel conversational. Most of my weekly mileage is too fast.'],
  ['running', 'Long run Sunday: 24 km at 5:40/km, fuelled every 40 minutes. Legs fine, calves tight after.'],
  ['coffee', 'Grind a bit finer for the light Ethiopian roast; it was sour and under-extracted at the usual setting.'],
  ['coffee', 'Bloom pour-over for 45 seconds with twice the coffee weight in water before the main pour.'],
  ['photography', 'Shoot street scenes at f/8 and zone focus at 3 metres; everything from 1.5 m to infinity is sharp.'],
  ['photography', 'The blue hour after sunset needs a tripod: 4 second exposures at ISO 100 kept the city lights clean.'],
  ['chess', 'I keep losing to the London System. Study the main lines with ...c5 and an early queen to b6.'],
  ['chess', 'In rook endgames, put the rook behind the passed pawn, whether it is yours or the opponent\'s.'],
  ['gardening', 'Tomatoes in bed 3 got blight again. Rotate them to bed 1 next year and mulch to stop soil splash.'],
  ['gardening', 'Basil and marigolds next to the tomatoes seemed to keep the aphids down this summer.'],
  ['astronomy', 'Saw Jupiter and four moons through the 8-inch dobsonian tonight. Seeing was steady after midnight.'],
  ['economics', 'If inflation stays above the target, expect the central bank to keep rates higher for longer.'],
  ['climate', 'Quote for the heat pump install came in. It should cut our gas use roughly in half.'],
  ['jazz', 'Practise transcribing one Chet Baker solo a week; sing it before playing it.'],
  ['ml', 'Validation loss went up after epoch 12 while training loss kept dropping: classic overfitting, add dropout.'],
  ['rome', 'Book idea: the late Republic as a story about institutions failing slowly, then all at once.'],
  ['geology', 'Visit Yellowstone in September: fewer crowds, and Old Faithful erupts roughly every 90 minutes.'],
]

/** [topic, url] — saved bare; the app reads the pages. */
const links = [
  ['rust', 'https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html'],
  ['rust', 'https://doc.rust-lang.org/book/ch16-01-threads.html'],
  ['ml', 'https://en.wikipedia.org/wiki/Generative_pre-trained_transformer'],
  ['memory', 'https://en.wikipedia.org/wiki/Spacing_effect'],
  ['astronomy', 'https://en.wikipedia.org/wiki/Event_Horizon_Telescope'],
  ['climate', 'https://en.wikipedia.org/wiki/Global_warming_potential'],
  ['jazz', 'https://en.wikipedia.org/wiki/A_Love_Supreme'],
  ['chess', 'https://en.wikipedia.org/wiki/AlphaZero'],
  ['rome', 'https://en.wikipedia.org/wiki/Battle_of_Actium'],
  ['coffee', 'https://en.wikipedia.org/wiki/Third-wave_coffee'],
  ['economics', 'https://en.wikipedia.org/wiki/Hyperinflation'],
  ['geology', 'https://en.wikipedia.org/wiki/Krakatoa'],
  ['baking', 'https://en.wikipedia.org/wiki/Pre-ferment'],
  ['photography', 'https://en.wikipedia.org/wiki/Aperture_priority'],
  ['running', 'https://en.wikipedia.org/wiki/Boston_Marathon'],
  ['gardening', 'https://en.wikipedia.org/wiki/No-dig_gardening'],
]

const cachePath = join(root, '.seed-cache.json')
mkdirSync(root, { recursive: true })
const cache = existsSync(cachePath) ? JSON.parse(readFileSync(cachePath, 'utf8')) : {}
const headers = { 'User-Agent': 'Plexus-seed/0.1 (https://github.com/sahilf637/plexus)' }

/** Short summary for most articles; every third gets the longer intro (several paragraphs). */
async function wikipedia(title, long) {
  const key = `${long ? 'intro' : 'summary'}:${title}`
  if (key in cache) return cache[key]
  const encoded = encodeURIComponent(title.replaceAll(' ', '_'))
  let text = null
  try {
    if (long) {
      const url = `https://en.wikipedia.org/w/api.php?action=query&prop=extracts&exintro&explaintext&redirects=1&format=json&titles=${encoded}`
      const pages = (await (await fetch(url, { headers })).json()).query?.pages ?? {}
      text = Object.values(pages)[0]?.extract ?? null
    } else {
      const response = await fetch(`https://en.wikipedia.org/api/rest_v1/page/summary/${encoded}`, { headers })
      if (response.ok) text = (await response.json()).extract ?? null
    }
  } catch (e) {
    console.warn(`  ${title}: ${e.message}`)
  }
  cache[key] = text?.trim() || null
  return cache[key]
}

// ULIDs sort by time; spread captures over the last month so the library looks lived-in.
const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'
function ulid(time) {
  let id = ''
  for (let t = time, i = 0; i < 10; i++, t = Math.floor(t / 32)) id = CROCKFORD[t % 32] + id
  for (const byte of randomBytes(16)) id += CROCKFORD[byte % 32]
  return id
}

const slug = (title) =>
  title
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 60)
    .replace(/-+$/, '')
const sha = (text) => 'sha256:' + createHash('sha256').update(text.replace(/\r\n/g, '\n').trim()).digest('hex')
const yaml = (value) => JSON.stringify(value)

const start = Date.parse('2026-09-05T08:00:00Z')
const notesToWrite = []
function add(topic, kind, title, body, extra = {}) {
  notesToWrite.push({ topic, kind, title, body, extra })
}

let fetched = 0
for (const [topic, titles] of Object.entries(articles)) {
  for (const [i, title] of titles.entries()) {
    const text = await wikipedia(title, i % 3 === 2)
    fetched++
    process.stdout.write(`\r  reading Wikipedia… ${fetched}`)
    if (text) add(topic, 'text', title, text)
  }
}
process.stdout.write('\n')
writeFileSync(cachePath, JSON.stringify(cache))

for (const [topic, language, snippet] of code) {
  const first = snippet.split('\n').find((l) => l.trim() && !l.trim().startsWith('#') && !l.trim().startsWith('//')) ?? 'Snippet'
  add(topic, 'code', first.trim().replace(/[{:]\s*$/, '').slice(0, 80), '```' + language + '\n' + snippet + '\n```', { language })
}
for (const [topic, note] of notes) add(topic, 'text', note.split(/(?<=[.!?:])\s/)[0].slice(0, 80), note)
for (const [topic, url] of links) add(topic, 'url', url.replace(/^https?:\/\/(www\.)?/, '').slice(0, 80), url, { source: url })

// Shuffle so topics arrive interleaved, as they would in real use.
for (let i = notesToWrite.length - 1; i > 0; i--) {
  const j = Math.floor(Math.random() * (i + 1))
  ;[notesToWrite[i], notesToWrite[j]] = [notesToWrite[j], notesToWrite[i]]
}

const step = (30 * 24 * 3600 * 1000) / notesToWrite.length
for (const [n, note] of notesToWrite.entries()) {
  const time = Math.round(start + n * step)
  const id = ulid(time)
  const lines = [
    '---',
    'plexus: 1',
    `id: ${id}`,
    `kind: ${note.kind}`,
    `title: ${yaml(note.title)}`,
    `captured: ${new Date(time).toISOString().replace(/\.\d+Z$/, 'Z')}`,
    ...(note.extra.source ? [`source: ${yaml(note.extra.source)}`] : []),
    ...(note.extra.language ? [`language: ${note.extra.language}`] : []),
    `hash: ${sha(note.body)}`,
    'links: []',
    `seed_topic: ${note.topic}`,
    '---',
    '',
    note.body,
    '',
  ]
  const stem = `${slug(note.title) || 'note'}-${id.slice(-4).toLowerCase()}`
  const date = new Date(time)
  const dir = join(itemsDir, String(date.getUTCFullYear()), String(date.getUTCMonth() + 1).padStart(2, '0'))
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `${stem}.md`), lines.join('\n'))
}

const counts = Object.groupBy(notesToWrite, (n) => n.kind)
console.log(
  `Wrote ${notesToWrite.length} notes to ${itemsDir} (` +
    Object.entries(counts).map(([k, v]) => `${v.length} ${k}`).join(', ') +
    `) across ${Object.keys(articles).length} topics.`,
)
console.log(`Start Plexus on it with:  PLEXUS_LIBRARY="${root}" npm run dev`)
