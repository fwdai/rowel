import { useEffect } from 'react'
import {
  useVault,
  selectCurrent,
  openPalette,
  openGenerator,
  openAddPicker,
  editEntry,
  focusSearch,
  isModalOpen,
  lockVault,
  setPref,
  SORT_MODES
} from '@/store'
import { SHORTCUTS } from '@/lib/shortcuts'
import { keyCode } from '@/utils/keys'

// The app-level shortcut surface. One listener, one record — a new chord is
// a key in `SHORTCUTS` and one line here. Mounted from Main, so chords are
// live only while unlocked.
//
// Keyed by physical key (`KeyboardEvent.code`), not by the character the layout
// prints, so the chords work the same on a Cyrillic keyboard (see `keyCode`).
const BINDINGS: Record<string, () => void> = {
  [SHORTCUTS.commands]: openPalette,
  [SHORTCUTS.lock]: lockVault,
  // Re-pressing ⌘G must not drop the callback the generator was opened with:
  // the flag flips before the card mounts, so one press cannot queue a second
  // open.
  [SHORTCUTS.generator]: () => {
    if (!isModalOpen()) openGenerator()
  },
  [SHORTCUTS.add]: openAddPicker,
  [SHORTCUTS.search]: focusSearch,
  // Edit whatever the list has selected — nothing to edit without a selection.
  [SHORTCUTS.edit]: () => {
    if (selectCurrent(useVault.getState())) editEntry()
  },
  ...Object.fromEntries(SORT_MODES.map((mode, i) => [`Digit${i + 1}`, () => setPref('sort', mode)]))
}

export const useShortcuts = () => {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return
      const run = BINDINGS[keyCode(e)]
      if (!run) return
      e.preventDefault()
      // A dialog owns the keyboard while it is up: every chord here would
      // otherwise act on the shell behind the scrim — opening a second modal,
      // pulling focus into the list column, or editing what the dialog covers.
      if (isModalOpen()) return
      run()
    }

    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}
