/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import { fileURLToPath, URL } from 'node:url'
import react from '@vitejs/plugin-react'
import svgr from 'vite-plugin-svgr'
import tailwindcss from '@tailwindcss/vite'

const host = process.env.TAURI_DEV_HOST
// The Tauri CLI exports this to the before*Command it runs ('ios', 'android',
// 'darwin', 'windows', 'linux'). Empty for a plain `vite`/`vitest` run, which
// reads as desktop — see src/lib/platform.ts.
const platform = process.env.TAURI_ENV_PLATFORM ?? ''

// https://vitejs.dev/config/ — tuned for Tauri (fixed port, no clearScreen).
export default defineConfig({
  plugins: [react(), svgr(), tailwindcss()],
  clearScreen: false,
  define: { __TAURI_PLATFORM__: JSON.stringify(platform) },
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) }
  },
  build: {
    // WKWebView on the minimum supported iOS (16) is Safari 16. Everywhere else
    // keeps Vite's default target, so desktop output is untouched.
    target: platform === 'ios' ? 'safari16' : undefined
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] }
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    css: true
  }
})
