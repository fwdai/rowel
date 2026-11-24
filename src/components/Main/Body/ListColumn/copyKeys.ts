import type { KeyboardEvent } from 'react'
import { useVault, selectCurrent, isModalOpen, currentShortcutKeys } from '@/store'
import { copyValue, type EntryValue } from '@/services/entries'
import { FIELD_KEYS, shortcutAt, type Shortcut } from '@/lib/shortcuts'
import { isPC } from '@/lib/platform'
import { keyCode } from '@/utils/keys'

// KeePass's copy chords, by action; their keys are `lib/shortcuts`. The palette
// shows them (Palette/commands.ts); the binding is this record.
const COPY_VALUES: Partial<Record<Shortcut, EntryValue>> = {
  copyUsername: 'username',
  copySecret: 'secret',
  copyOtp: 'otp',
  copySecretOtp: 'secretOtp',
  copyUrl: 'url'
}

/**
 * The value a copy chord asks for, or undefined for any other key. `inField`
 * leaves out the keys a text field owns (`FIELD_KEYS`): from the search field
 * ⌘C is the query's own copy and Ctrl+Y a PC's redo, and the field's ⌘⏎ covers
 * the password. It also wants the platform's own modifier there: in a Mac text
 * field Ctrl is the caret's (⌃B back a character, ⌃T transpose), so only ⌘
 * copies.
 */
export const copyChord = (event: globalThis.KeyboardEvent, inField = false) => {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return
  if (inField && !(isPC ? event.ctrlKey : event.metaKey)) return
  const code = keyCode(event)
  if (inField && FIELD_KEYS.includes(code)) return
  const action = shortcutAt(currentShortcutKeys(), code)
  return action && COPY_VALUES[action]
}

// Bound to the listbox, and live only on a row: clicking a row focuses it, so
// that is where the keyboard sits once an entry is picked. The search field
// takes the chords it has no use for (see `copyChord`); everywhere else — the
// detail pane, any editor — ⌘C stays the system copy.
export const onCopyKey = (event: KeyboardEvent<HTMLElement>) => {
  const value = copyChord(event.nativeEvent)
  if (!value) return
  if (!(event.target instanceof Element && event.target.closest('[role="option"]'))) return
  // Text the user has selected is what they mean ⌘C to take.
  if (window.getSelection()?.toString()) return
  if (isModalOpen()) return
  const entry = selectCurrent(useVault.getState())
  if (!entry) return
  event.preventDefault()
  void copyValue(entry, value)
}
