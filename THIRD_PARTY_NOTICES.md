# Third-party notices

Plexus includes or adapts the following third-party work.

## Atomic

`backend/src/pipeline/chunk.rs` is adapted from Atomic's markdown chunker
(`crates/atomic-core/src/chunking.rs`, https://github.com/kenforthewin/atomic).

```
MIT License

Copyright (c) 2025 Kenneth Bergquist

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## bge-small-en-v1.5

The bundled embedding model, by BAAI (https://huggingface.co/BAAI/bge-small-en-v1.5), MIT
license; ONNX conversion by Xenova (https://huggingface.co/Xenova/bge-small-en-v1.5). Downloaded
at build time by `scripts/fetch-model.mjs`.

## ONNX Runtime

Runs the embedding model. Microsoft, MIT license (https://github.com/microsoft/onnxruntime),
linked through the `ort` crate (MIT/Apache-2.0).

## PP-OCRv6 text recognition models

The bundled OCR models (PP-OCRv6 small detector and recogniser, and their dictionary) by
PaddlePaddle (https://github.com/PaddlePaddle/PaddleOCR), Apache-2.0, as converted to ONNX by
RapidOCR (https://github.com/RapidAI/RapidOCR), Apache-2.0, and run through the `rapidocr-core`
crate. Downloaded at build time by `scripts/fetch-model.mjs`.
