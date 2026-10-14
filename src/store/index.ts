/**
 * Frontend state, split by lifetime rather than by feature:
 *
 * - `prefs` — user preferences; persisted, never wiped.
 * - `app`   — flow, lock-screen gate, backend sync/setup status, staged update;
 *             lives as long as the process, survives a lock.
 * - `vault` — the unlocked vault's rows, selection and audit; wiped on lock.
 * - `ui`    — surfaces open over the vault and the list's filters; wiped on lock.
 *
 * Each is a plain zustand store whose actions are exported module functions, so
 * they are called the same way from a component, a keyboard shortcut, a Tauri
 * event handler or a test. Rust owns the vault, the session and sync; these
 * stores hold what it last said plus what is on screen.
 */
export * from './prefs'
export * from './app'
export * from './vault'
export * from './ui'
export * from './workspace'
export * from './account'
