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
  currentShortcutKeys,
  SORT_MODES
} from '@/store'
import { shortcutAt, type Shortcut } from '@/lib/shortcuts'
import { keyCode } from '@/utils/keys'

// The app-level shortcut surface. One listener, one record — a new chord is
// an action in `lib/shortcuts` and one line here. Mounted from Main, so chords
// are live only while unlocked.
//
// The key each action is on is looked up per press (`currentShortcutKeys`), so
// a change in Settings › Keyboard shortcuts applies at once.
const ACTIONS: Partial<Record<Shortcut, () => void>> = {
  commands: openPalette,
  lock: lockVault,
  // Re-pressing ⌘G must not drop the callback the generator was opened with:
  // the flag flips before the card mounts, so one press cannot queue a second
  // open.
  generator: () => {
    if (!isModalOpen()) openGenerator()
  },
  add: openAddPicker,
  search: focusSearch,
  // Edit whatever the list has selected — nothing to edit without a selection.
  edit: () => {
    if (selectCurrent(useVault.getState())) editEntry()
  }
}

// ⌘1…⌘n pick the sort modes in menu order. Fixed: the user remaps letters only.
const SORT_KEYS: Record<string, () => void> = Object.fromEntries(
  SORT_MODES.map((mode, i) => [`Digit${i + 1}`, () => setPref('sort', mode)])
)

// Keyed by physical key (`KeyboardEvent.code`), not by the character the layout
// prints, so the chords work the same on a Cyrillic keyboard (see `keyCode`).
const bindingFor = (code: string) => {
  const action = shortcutAt(currentShortcutKeys(), code)
  return (action && ACTIONS[action]) || SORT_KEYS[code]
}

export const useShortcuts = () => {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return
      const run = bindingFor(keyCode(e))
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
