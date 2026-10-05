// Downloads the embedding model Plexus ships with (it links items by meaning, offline) into
// backend/resources/models/bge-small-en-v1.5/, checking every file's SHA-256.
//
// Runs before `npm run dev` and `npm run build`. Files already present and correct are skipped,
// so rerunning it is cheap.
//
// Model: bge-small-en-v1.5 (MIT, BAAI), int8-quantized ONNX conversion by Xenova, pinned to one
// commit: https://huggingface.co/Xenova/bge-small-en-v1.5

import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const COMMIT = 'ea104dacec62c0de699686887e3f920caeb4f3e3'
const BASE = `https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/${COMMIT}`

/** [path in the repo, local name, sha256] */
const FILES = [
  ['onnx/model_quantized.onnx', 'model.onnx', '6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4'],
  ['tokenizer.json', 'tokenizer.json', 'd241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66'],
  ['config.json', 'config.json', 'fa73f90bf92c8cace1fbcb709626306f2bdbc9ea3e5b5f94b440df9b6aa56350'],
  ['special_tokens_map.json', 'special_tokens_map.json', 'b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3'],
  ['tokenizer_config.json', 'tokenizer_config.json', '9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3'],
]

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const outDir = join(root, 'backend', 'resources', 'models', 'bge-small-en-v1.5')
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')

await mkdir(outDir, { recursive: true })
for (const [source, name, expected] of FILES) {
  const dest = join(outDir, name)
  if (existsSync(dest) && sha256(await readFile(dest)) === expected) continue

  process.stdout.write(`Fetching ${name}… `)
  const response = await fetch(`${BASE}/${source}`)
  if (!response.ok) throw new Error(`${source}: HTTP ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  const actual = sha256(bytes)
  if (actual !== expected) throw new Error(`${source}: checksum mismatch (got ${actual})`)
  // Write then rename, so an interrupted download never leaves a bad file in place.
  await writeFile(`${dest}.partial`, bytes)
  await rename(`${dest}.partial`, dest)
  console.log(`${(bytes.length / 1024 / 1024).toFixed(1)} MB`)
}
