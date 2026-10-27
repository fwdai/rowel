import { invoke } from '@tauri-apps/api/core'
import type { AppStatus, Settings } from '@/api/app'

/**
 * Dev-only bridge to the backend's `e2e_reset` and `set_settings` commands, so
 * a spec can put the vault and the preferences into a known state between
 * tests, and to the switches an agent's ask needs.
 *
 * Zero production footprint: the only caller guards on `import.meta.env.DEV`
 * and imports this module dynamically, so Vite folds the branch to `false` and
 * drops the whole module from a production bundle. The command it calls is in
 * turn compiled out of release binaries (see `src-tauri/src/commands/e2e.rs`).
 *
 * The bridge deliberately does NOT reload the page: navigating while a
 * WebDriver script is still resolving tears the execution context out from
 * under the driver. The spec helper reloads through `browser.refresh()`
 * instead, which waits for the new document properly.
 */

export type E2EResetMode = 'pristine' | 'empty'

declare global {
  interface Window {
    __e2eReset?: (mode: E2EResetMode, password?: string) => Promise<void>
    __e2eSettings?: (patch?: Partial<Settings>) => Promise<Settings>
    __e2eAgents?: () => Promise<void>
  }
}

export function installE2EBridge(): void {
  window.__e2eReset = (mode, password) =>
    invoke('e2e_reset', { mode, password: password ?? null })

  // Preferences live in `settings.json` now, where the driver cannot reach
  // them. With a patch this writes one; without, it reads them all back — which
  // is how a spec proves a row persisted rather than merely redrew.
  window.__e2eSettings = patch =>
    patch
      ? invoke<Settings>('set_settings', { patch })
      : invoke<AppStatus>('app_status').then(status => status.settings)

  // The three switches an agent's ask needs, as their Settings rows flip
  // them: the extension host listening, the Agents switch, and the open
  // primary lockbox's agent access.
  window.__e2eAgents = async () => {
    await invoke('browser_set_enabled', { enabled: true })
    await invoke('browser_set_agents_enabled', { enabled: true })
    await invoke('workspace_set_agents', { id: 'default', enabled: true })
  }
}
