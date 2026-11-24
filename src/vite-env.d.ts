/// <reference types="vite/client" />

// Baked in by vite.config.ts from the Tauri CLI's TAURI_ENV_PLATFORM.
declare const __TAURI_PLATFORM__: string

/**
 * What the first painted frame needs, injected by `src-tauri/src/window.rs`
 * before any script of ours runs (see `commands::app::Boot`): the theme, inside
 * the settings, and the language to load the catalogue for. Absent under vitest
 * and on any non-Tauri host, which is why `boot.ts` validates rather than
 * trusts it.
 */
interface RowelBoot {
  locale: string
  settings: import('@/api/app').Settings
}

interface Window {
  __ROWEL_BOOT__?: RowelBoot
  /** The Android activity's side of the shell; see `lib/android`. */
  __ROWEL_ANDROID__?: import('@/lib/android').AndroidShell
}
