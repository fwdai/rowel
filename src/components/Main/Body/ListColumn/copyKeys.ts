import type { KeyboardEvent } from 'react'
import { useVault, selectCurrent, isModalOpen } from '@/store'
import { copyValue, type EntryValue } from '@/services/entries'
import { keyCode } from '@/utils/keys'

// KeePass's copy chords, by physical key (see `keyCode`). The palette shows
// them (Palette/commands.ts); the binding is this record.
const COPY_KEYS: Record<string, EntryValue> = {
  KeyB: 'username',
  KeyC: 'secret',
  KeyT: 'otp',
  KeyU: 'url'
}

// Bound to the listbox, and live only on a row: clicking a row focuses it, so
// that is where the keyboard sits once an entry is picked. Everywhere else —
// the search field, the detail pane, any editor — ⌘C stays the system copy.
export const onCopyKey = (event: KeyboardEvent<HTMLElement>) => {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return
  const value = COPY_KEYS[keyCode(event.nativeEvent)]
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
