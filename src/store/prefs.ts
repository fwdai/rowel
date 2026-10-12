import { create } from 'zustand'
import {
  DATE_FORMATS,
  SORT_MODES,
  setSettings,
  type DateFormat,
  type Settings,
  type SortMode
} from '@/api/app'
import i18n from '@/i18n'
import {
  applyAccent,
  applyTheme,
  isAccent,
  resolveTheme,
  DEFAULT_ACCENT,
  type Theme,
  type ThemePreference
} from '@/theme'

/**
 * User preferences: everything that outlives a session and a lock.
 *
 * Rust owns the file (`settings.json` beside the other sidecars, see
 * `src-tauri/src/settings.rs`); this is the webview's copy of it, hydrated once
 * at boot from `app_status` and written back through `set_settings`. One store,
 * so a preference is read like any other state (`usePrefs(s => s.sort)`) and a
 * change re-renders whoever shows it — nothing copies a preference into local
 * state, and nothing reads it from disk at call time.
 */

export type { DateFormat, SortMode, Settings as Prefs } from '@/api/app'
export { DATE_FORMATS, SORT_MODES } from '@/api/app'

// The same defaults as Rust, so a test store — and the split second before
// hydration lands — reads like a fresh install rather than like `undefined`.
export const DEFAULT_PREFS: Settings = {
  theme: 'light',
  accent: DEFAULT_ACCENT,
  sort: 'recent',
  breachCheck: false,
  autolockSecs: 60,
  clipboardTimeoutMs: 30_000,
  hideOnCopy: false,
  dateFormat: 'MM/DD/YYYY',
  locale: null,
  generator: {
    length: 20,
    numbers: true,
    symbols: true,
    uppercase: true,
    lowercase: true,
    exclude: '',
    excludeSimilarCharacters: false
  }
}

const isTheme = (value: unknown): value is ThemePreference =>
  value === 'light' || value === 'dark' || value === 'system'

const isPositive = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value) && value > 0

/**
 * The file is user-writable and may predate a knob added since, so each field
 * is checked and falls back to its default rather than trusted as a whole
 * shape. `generator` is merged one level deep for the same reason. Rust stores
 * the enum-ish fields as plain strings; this is where they are narrowed.
 */
const sanitize = (raw: Partial<Settings>): Settings => {
  const generator = { ...DEFAULT_PREFS.generator, ...(raw.generator ?? {}) }
  return {
    theme: isTheme(raw.theme) ? raw.theme : DEFAULT_PREFS.theme,
    accent: isAccent(raw.accent) ? raw.accent : DEFAULT_PREFS.accent,
    sort: SORT_MODES.includes(raw.sort as SortMode) ? (raw.sort as SortMode) : 'recent',
    breachCheck: raw.breachCheck === true,
    autolockSecs: isPositive(raw.autolockSecs) ? raw.autolockSecs : DEFAULT_PREFS.autolockSecs,
    clipboardTimeoutMs:
      typeof raw.clipboardTimeoutMs === 'number' &&
      Number.isFinite(raw.clipboardTimeoutMs) &&
      raw.clipboardTimeoutMs >= 0
        ? raw.clipboardTimeoutMs
        : DEFAULT_PREFS.clipboardTimeoutMs,
    hideOnCopy: raw.hideOnCopy === true,
    dateFormat: DATE_FORMATS.includes(raw.dateFormat as DateFormat)
      ? (raw.dateFormat as DateFormat)
      : DEFAULT_PREFS.dateFormat,
    locale: typeof raw.locale === 'string' ? raw.locale : null,
    generator: Number.isFinite(generator.length)
      ? generator
      : { ...generator, length: DEFAULT_PREFS.generator.length }
  }
}

export const usePrefs = create<Settings>()(() => DEFAULT_PREFS)

/** Take the boot probe's answer wholesale (see `boot.ts`). */
export const hydratePrefs = (settings: Settings): void => {
  usePrefs.setState(sanitize(settings), true)
}

/**
 * Patch one preference. The new value lands immediately — a toggle must not
 * wait on a file write to look pressed — and is then replaced by what Rust
 * merged, which is authoritative. A failed write leaves the optimistic value in
 * place: the preference still applies for this session, it just won't survive
 * a restart.
 *
 * Authoritative for the keys it carries: the answer is laid over the store
 * rather than swapped in for it, so a backend built before a preference
 * existed — which merges the patch into a struct that has no field for it and
 * answers without the key — leaves the choice standing for the session instead
 * of snapping it back to the default on every click. Same outcome as the
 * failed write above, for the same reason.
 *
 * Except for what is still being written. Two writes to different keys
 * overlap all the time (an accent, then a theme), and each answer is the whole
 * file as the backend held it when *that* patch was merged — so the theme's
 * answer, landing first, may still carry the old accent. Every key with an
 * unanswered write keeps its optimistic value over any answer; its own answer,
 * or a failure, is what releases it.
 */
// Which write is the newest. Answers can land out of order — a slider drag
// issues several in a row — and each one carries the whole file, so only the
// answer to the latest write may replace the store: an older one would put back
// a value the user has already moved past.
let latestWrite = 0
// Per key, the newest write still waiting for its answer and the value it
// carries: the overlay every answer is read through.
const inFlight = new Map<keyof Settings, { write: number; value: Settings[keyof Settings] }>()

const pendingOverlay = (except: number): Partial<Settings> =>
  Object.fromEntries(
    [...inFlight].filter(([, w]) => w.write !== except).map(([key, w]) => [key, w.value])
  )

export const setPref = <K extends keyof Settings>(key: K, value: Settings[K]): void => {
  const write = ++latestWrite
  inFlight.set(key, { write, value })
  usePrefs.setState({ [key]: value } as Pick<Settings, K>)
  setSettings({ [key]: value } as Partial<Settings>)
    .then(merged => {
      if (write === latestWrite)
        usePrefs.setState(
          sanitize({ ...usePrefs.getState(), ...merged, ...pendingOverlay(write) }),
          true
        )
    })
    .catch(() => {})
    .finally(() => {
      // Only this write's own entry: a newer write to the same key has
      // replaced it and is still owed an answer.
      if (inFlight.get(key)?.write === write) inFlight.delete(key)
    })
}

/**
 * A stamp of the write history, taken before a probe is sent, so its answer
 * can tell whether it is still the newest word (`hydrateFromProbe`).
 */
export const prefsMark = (): number => latestWrite

/**
 * Take a probe's settings only when they cannot be older than what the store
 * holds: no write was issued after the probe went out, and none is still
 * waiting for its answer. A probe that raced a write — `app_status` read the
 * file before `set_settings` rewrote it, then landed after — would otherwise
 * put the old value back over a change the backend has already kept. When a
 * write is pending its answer is the newer fact and carries the whole file
 * anyway, so nothing is lost by letting the probe's copy go.
 */
export const hydrateFromProbe = (settings: Settings, mark: number): void => {
  if (mark === latestWrite && inFlight.size === 0) hydratePrefs(settings)
}

// The palette command is a flip, so it resolves "system" first and then lands
// on a concrete light/dark preference.
export const toggleTheme = () => {
  const next: Theme = resolveTheme(usePrefs.getState().theme) === 'dark' ? 'light' : 'dark'
  setPref('theme', next)
}

// The theme is the one preference with a side effect outside React: the
// document root's `data-theme`, which theme.css keys its tokens off.
usePrefs.subscribe((state, previous) => {
  if (state.theme !== previous.theme) applyTheme(state.theme)
  if (state.accent !== previous.accent) applyAccent(state.accent)
})

// "System" has to keep following the OS, not just read it once at startup. No
// matchMedia (jsdom) or a locked-down webview simply means the preference stops
// tracking, which is what it did before anyway.
try {
  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (usePrefs.getState().theme === 'system') applyTheme('system')
  })
} catch {
  // No subscription; "system" stays on whatever it resolved to at load.
}

// The language is changed through i18next rather than through a row, so this
// is where that choice becomes a stored preference — one listener, so a change
// from anywhere is persisted the same way. The document's own `lang` is i18n's
// half of the same event.
//
// Not during init: i18next fires the same event while starting in the locale
// Rust resolved, and writing that back would pin an OS-following install
// (`locale: null`) to whatever the OS said on first boot. Only a change made
// once the app is up is a choice worth keeping.
i18n.on('languageChanged', locale => {
  if (i18n.isInitialized && locale !== usePrefs.getState().locale) setPref('locale', locale)
})
