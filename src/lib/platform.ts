// Which Tauri target this bundle was built for. `__TAURI_PLATFORM__` is baked
// in by vite.config.ts from the CLI's TAURI_ENV_PLATFORM, so these are compile
// -time constants rather than a user-agent guess: the same WKWebView serves
// macOS and iOS, and its UA cannot tell them apart.
export const isIOS = __TAURI_PLATFORM__ === 'ios'
export const isAndroid = __TAURI_PLATFORM__ === 'android'
export const isMobile = isIOS || isAndroid

// A PC — the one place the app's chords are held with Ctrl (`useShortcuts`
// takes either modifier). A plain vite/vitest run bakes in `''`, which reads as
// the Apple default, like every other unset platform.
export const isPC = __TAURI_PLATFORM__ === 'windows' || __TAURI_PLATFORM__ === 'linux'

/**
 * A shortcut hint for `key` as this platform spells it: `⌘F` on Apple
 * hardware, `Ctrl+F` on a PC. Every hint chip in the app (`Kbd`, the empty
 * states' hint rows) should go through here rather than hardcoding ⌘.
 */
export const chord = (key: string) => (isPC ? `Ctrl+${key}` : `⌘${key}`)
