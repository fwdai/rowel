import { createElement, StrictMode } from 'react'
import ReactDOM from 'react-dom/client'
import App from '@/App'
import { appReady, appStatus, type AppStatus } from '@/api/app'
import { flowSetup, hydratePrefs, setApp, setUpdateReady, usePrefs } from '@/store'
import { runStartupUpdateCheck } from '@/api/autoUpdate'
import { applyAccent, applyTheme } from '@/theme'
import { changeLocale, DEFAULT_LOCALE, getLocale, initI18n } from '@/i18n'
import { followSystemInsets } from '@/lib/android'
import { startSplash } from '@/lib/splash'

/**
 * Launch, from the injected boot payload to React's first commit.
 *
 * Two things gate the first frame and they run side by side: the catalogue for
 * the language, and the probe that says which flow to open on. Neither may keep
 * the app off screen if it fails — a host without a backend still gets a lock
 * screen, which re-probes on its own (see `App.tsx`).
 *
 * The splash is not one of the gates. It is armed synchronously, the window is
 * revealed on the same tick, and React takes over whenever it is ready: the
 * lock screen draws the same mascot in the same pixels, so a handover
 * mid-choreography reads as a cross-fade rather than a cut.
 */

// Long enough to be after the first commit on any machine, short enough that a
// staged update is still found in the session it was published in. Off the boot
// path entirely: the check makes a network request and writes to disk, and
// nothing on screen waits for it.
const UPDATE_CHECK_DELAY_MS = 2_000

/**
 * The injected payload, or null on any host that did not inject one (vitest, a
 * plain browser). Checked field by field rather than trusted: it is the one
 * input that arrives before the app can have validated anything.
 */
const injectedBoot = (): RowelBoot | null => {
  const boot: unknown = window.__ROWEL_BOOT__
  if (!boot || typeof boot !== 'object') return null
  const { locale, settings } = boot as Partial<RowelBoot>
  if (typeof locale !== 'string' || !settings || typeof settings !== 'object') return null
  return { locale, settings }
}

/**
 * Take a probe's answer into the stores (`setApp` hydrates the preferences),
 * then paint the document even where nothing changed: on a fresh page the root
 * carries no theme or accent yet, so a stored value equal to the store's
 * default would otherwise never reach it.
 */
const adopt = (status: AppStatus) => {
  setApp(status)
  applyTheme(usePrefs.getState().theme)
  applyAccent(usePrefs.getState().accent)
}

/**
 * The preferences a build before Rust owned `settings.json` kept in
 * localStorage, carried into the file once and the keys removed (see
 * `lib/legacyPrefs`). Behind a dynamic import and after the first commit, so
 * the launch chunk does not carry it and nothing on screen waits for it; on
 * every install that has nothing to import it reads a few absent keys and
 * stops. When something was carried over the probe is re-read, since only Rust
 * can say what the stored language resolves to.
 */
const adoptLegacyPrefs = async (status: AppStatus) => {
  const { adoptLegacyPrefs: adoptLegacy } = await import('@/lib/legacyPrefs')
  const after = await adoptLegacy(status)
  if (after === status) return
  adopt(after)
  if (after.locale !== getLocale()) await changeLocale(after.locale)
}

export const boot = async (): Promise<void> => {
  // Before the splash is armed and the window revealed, so the choreography
  // plays clear of the screen's edges from its first frame.
  followSystemInsets()
  const injected = injectedBoot()
  if (injected) {
    // Before anything can await: the first frame React paints is then already
    // in the stored theme rather than in the default it would start from.
    hydratePrefs(injected.settings)
    applyTheme(usePrefs.getState().theme)
    applyAccent(usePrefs.getState().accent)
  }
  const locale = injected?.locale ?? DEFAULT_LOCALE

  startSplash()
  // Reveals the window. A failure here only means the fallback timer does it.
  void appReady().catch(() => {})

  const translated = initI18n(locale).catch(() => {})
  const probed: Promise<AppStatus | null> = appStatus()
    .then(status => {
      adopt(status)
      return status
    })
    // `status` stays null and the shell opens on the lock screen; nothing about
    // a dead probe may leave the app unrendered.
    .catch(() => null)

  const [, status] = await Promise.all([translated, probed])
  // Only reachable when nothing was injected: with a payload, Rust resolved the
  // language before the bundle ran and the catalogue is already the right one.
  const resolved = status?.locale ?? locale
  if (resolved !== locale) await changeLocale(resolved).catch(() => {})
  // Nothing on disk is the one answer that opens somewhere other than the lock
  // screen. Decided before the first render, so the flow that commits first is
  // the one the user will see — and the splash stays up over the setup chunk
  // while it loads rather than fading off a lock screen that is about to go.
  if (status?.initialized === false) flowSetup()

  ReactDOM.createRoot(document.getElementById('root')!).render(
    createElement(StrictMode, null, createElement(App))
  )

  if (status) void adoptLegacyPrefs(status).catch(() => {})
  setTimeout(() => void runStartupUpdateCheck(setUpdateReady), UPDATE_CHECK_DELAY_MS)
}
