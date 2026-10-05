// Downloads the models Plexus ships with into backend/resources/models/: the embedding model
// (links items by meaning, offline) and the OCR models (read text in images), checking every
// file's SHA-256.
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

// OCR models for reading text in images: PaddleOCR PP-OCRv6 small (Apache-2.0), as packaged by
// RapidOCR (https://github.com/RapidAI/RapidOCR). Listed with the same names `rapidocr-core` expects.
const OCR_BASE = 'https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve'
const OCR_FILES = [
  ['v3.9.0/onnx/PP-OCRv6/det/PP-OCRv6_det_small.onnx', 'PP-OCRv6_det_small.onnx', '090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f'],
  ['v3.9.0/onnx/PP-OCRv6/rec/PP-OCRv6_rec_small.onnx', 'PP-OCRv6_rec_small.onnx', '6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884'],
  ['master/paddle/PP-OCRv6/rec/PP-OCRv6_rec_small/ppocrv6_dict.txt', 'ppocrv6_dict.txt', 'b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d'],
]

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')

const downloads = [
  ...FILES.map(([source, name, sha]) => [join(root, 'backend', 'resources', 'models', 'bge-small-en-v1.5'), name, `${BASE}/${source}`, sha]),
  ...OCR_FILES.map(([path, name, sha]) => [join(root, 'backend', 'resources', 'models', 'ppocr'), name, `${OCR_BASE}/${path}`, sha]),
]

for (const [outDir, name, url, expected] of downloads) {
  await mkdir(outDir, { recursive: true })
  const dest = join(outDir, name)
  if (existsSync(dest) && sha256(await readFile(dest)) === expected) continue

  process.stdout.write(`Fetching ${name}… `)
  const response = await fetch(url)
  if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  const actual = sha256(bytes)
  if (actual !== expected) throw new Error(`${name}: checksum mismatch (got ${actual})`)
  // Write then rename, so an interrupted download never leaves a bad file in place.
  await writeFile(`${dest}.partial`, bytes)
  await rename(`${dest}.partial`, dest)
  console.log(`${(bytes.length / 1024 / 1024).toFixed(1)} MB`)
}
