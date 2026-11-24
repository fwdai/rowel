import type { TKey } from '@/i18n'
import { chord } from './platform'

/**
 * The app's ⌘/Ctrl + letter chords: each action's default key (a physical
 * `KeyboardEvent.code`, see `utils/keys`) and its name in Settings › Keyboard
 * shortcuts, in the order Settings lists them.
 *
 * The user's changes live in the `shortcuts` preference and are laid over
 * these by `resolveShortcuts`; the bindings (`Main/useShortcuts`, the list's
 * `copyKeys`) and every hint read the result, so a hint always names the key
 * that is bound.
 */
export const SHORTCUTS = {
  commands: { key: 'KeyK', label: 'Open the command palette' },
  search: { key: 'KeyF', label: 'Search' },
  add: { key: 'KeyN', label: 'Add a secret' },
  edit: { key: 'KeyE', label: 'Edit the selected item' },
  generator: { key: 'KeyG', label: 'Generate a password' },
  lock: { key: 'KeyL', label: 'Lock vault' },
  copyUsername: { key: 'KeyB', label: 'Copy username' },
  copySecret: { key: 'KeyC', label: 'Copy password' },
  copyOtp: { key: 'KeyT', label: 'Copy one-time code' },
  copySecretOtp: { key: 'KeyY', label: 'Copy password and one-time code' },
  copyUrl: { key: 'KeyU', label: 'Copy URL' }
} as const satisfies Record<string, { key: string; label: TKey }>

export type Shortcut = keyof typeof SHORTCUTS

export type ShortcutKeys = Record<Shortcut, string>

/** What the preference holds: only the actions the user moved. */
export type ShortcutOverrides = Partial<ShortcutKeys>

export const SHORTCUT_IDS = Object.keys(SHORTCUTS) as Shortcut[]

export const resolveShortcuts = (overrides: ShortcutOverrides): ShortcutKeys => ({
  ...(Object.fromEntries(SHORTCUT_IDS.map(id => [id, SHORTCUTS[id].key])) as ShortcutKeys),
  ...overrides
})

/** The action a key runs, if any. */
export const shortcutAt = (keys: ShortcutKeys, code: string): Shortcut | undefined =>
  SHORTCUT_IDS.find(id => keys[id] === code)

/** How a key is written on a hint chip: `⌘K` on Apple hardware, `Ctrl+K` on a PC. */
export const hintOf = (code: string) => chord(code.slice('Key'.length))

// Taken by the OS on every desktop: quit, close, hide, minimise.
const SYSTEM_KEYS = ['KeyQ', 'KeyW', 'KeyH', 'KeyM']

/** A text field's own chords: select all, copy, paste, cut, undo, and a PC's redo. */
export const FIELD_KEYS = ['KeyA', 'KeyC', 'KeyV', 'KeyX', 'KeyZ', 'KeyY']

/**
 * Why `code` can't be `id`'s key, or undefined when it can. Letters only —
 * ⌘1…⌘n are the sort modes'. A text field's chords stay with the field, except
 * as the default they already are: ⌘C and ⌘Y copy from a list row, where no
 * field is listening.
 */
export const refusal = (id: Shortcut, code: string): 'invalid' | 'system' | 'editing' | undefined => {
  if (!/^Key[A-Z]$/.test(code)) return 'invalid'
  if (SYSTEM_KEYS.includes(code)) return 'system'
  if (FIELD_KEYS.includes(code) && code !== SHORTCUTS[id].key) return 'editing'
  return undefined
}

/** Move `id` to `code`; an action back on its default drops out of the preference. */
export const assignShortcut = (
  overrides: ShortcutOverrides,
  id: Shortcut,
  code: string
): ShortcutOverrides => {
  const next = { ...overrides, [id]: code }
  if (code === SHORTCUTS[id].key) delete next[id]
  return next
}

/** The preference as read from a user-writable file: known actions with a key. */
export const sanitizeShortcuts = (raw: unknown): ShortcutOverrides =>
  raw && typeof raw === 'object'
    ? Object.fromEntries(
        Object.entries(raw).filter(
          ([id, code]) =>
            Object.prototype.hasOwnProperty.call(SHORTCUTS, id) && typeof code === 'string'
        )
      )
    : {}
