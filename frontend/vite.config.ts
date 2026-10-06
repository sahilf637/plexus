import { readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig, type Plugin } from 'vite'

/** pdf.js loads fonts, character maps and image decoders on demand; they're served from
 * `/pdfjs/<folder>/` (copied into the build, served from node_modules in dev). */
function pdfjsAssets(): Plugin {
  const root = dirname(createRequire(import.meta.url).resolve('pdfjs-dist/package.json'))
  const folders = ['standard_fonts', 'cmaps', 'wasm']
  return {
    name: 'pdfjs-assets',
    configureServer(server) {
      server.middlewares.use('/pdfjs/', (req, res, next) => {
        const [folder, file] = (req.url ?? '').split('?')[0].replace(/^\//, '').split('/')
        if (!folders.includes(folder) || !file || file.includes('..')) return next()
        try {
          res.end(readFileSync(join(root, folder, file)))
        } catch {
          next()
        }
      })
    },
    generateBundle() {
      for (const folder of folders)
        for (const file of readdirSync(join(root, folder)))
          this.emitFile({ type: 'asset', fileName: `pdfjs/${folder}/${file}`, source: readFileSync(join(root, folder, file)) })
    },
  }
}

// Tauri expects a fixed dev port and handles its own logging.
// https://v2.tauri.app/start/frontend/vite/
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [svelte(), pdfjsAssets()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1431 } : undefined,
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // Two windows, two pages: the map and the capture window.
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, 'index.html'),
        capture: resolve(import.meta.dirname, 'capture.html'),
      },
    },
    // Tauri uses Chromium (WebView2) on Windows and WebKit on macOS/Linux.
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari15',
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
})
