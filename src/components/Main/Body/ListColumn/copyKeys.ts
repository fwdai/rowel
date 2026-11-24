import type { KeyboardEvent } from 'react'
import { useVault, selectCurrent, isModalOpen } from '@/store'
import { copyValue, type EntryValue } from '@/services/entries'
import { SHORTCUTS } from '@/lib/shortcuts'
import { isPC } from '@/lib/platform'
import { keyCode } from '@/utils/keys'

// KeePass's copy chords. The palette shows them (Palette/commands.ts); the
// binding is this record.
const COPY_KEYS: Record<string, EntryValue> = {
  [SHORTCUTS.copyUsername]: 'username',
  [SHORTCUTS.copySecret]: 'secret',
  [SHORTCUTS.copyOtp]: 'otp',
  [SHORTCUTS.copySecretOtp]: 'secretOtp',
  [SHORTCUTS.copyUrl]: 'url'
}

// The copy chords a text field has no use for, which answer from the search
// field as well (`useListKeys`). ⌘C there is the query's own copy and Ctrl+Y a
// PC's redo, so those two stay on the row; the field's ⌘⏎ covers the password.
const FIELD_KEYS = new Set<string>([
  SHORTCUTS.copyUsername,
  SHORTCUTS.copyOtp,
  SHORTCUTS.copyUrl
])

/**
 * The value a copy chord asks for, or undefined for any other key. `inField`
 * narrows it to the chords the search field leaves alone, held with the
 * platform's own modifier: in a Mac text field Ctrl is the caret's (⌃B back a
 * character, ⌃T transpose), so only ⌘ copies there.
 */
export const copyChord = (event: globalThis.KeyboardEvent, inField = false) => {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return
  if (inField && !(isPC ? event.ctrlKey : event.metaKey)) return
  const code = keyCode(event)
  if (inField && !FIELD_KEYS.has(code)) return
  return COPY_KEYS[code]
}

// Bound to the listbox, and live only on a row: clicking a row focuses it, so
// that is where the keyboard sits once an entry is picked. The search field
// takes the chords it has no use for (see `FIELD_KEYS`); everywhere else — the
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
