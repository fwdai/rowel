import { chord } from './platform'

/**
 * The app's ⌘/Ctrl + letter chords, each by the physical key it is bound to (a
 * `KeyboardEvent.code`, see `utils/keys`). The bindings — `Main/useShortcuts`
 * and the list's `copyKeys` — and every hint that names a chord read this one
 * record, so a hint can't show a key the app doesn't bind.
 */
export const SHORTCUTS = {
  commands: 'KeyK',
  lock: 'KeyL',
  generator: 'KeyG',
  add: 'KeyN',
  search: 'KeyF',
  edit: 'KeyE',
  copyUsername: 'KeyB',
  copySecret: 'KeyC',
  copyOtp: 'KeyT',
  copySecretOtp: 'KeyY',
  copyUrl: 'KeyU'
} as const

export type Shortcut = keyof typeof SHORTCUTS

/** How a chord is written on a hint chip: `⌘K` on Apple hardware, `Ctrl+K` on a PC. */
export const shortcutHint = (id: Shortcut) => chord(SHORTCUTS[id].slice('Key'.length))
