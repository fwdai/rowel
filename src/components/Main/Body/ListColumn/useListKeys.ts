import type { KeyboardEvent } from 'react'
import { useVault, setCurrentEntry, pickAll, pickRange } from '@/store'
import { copySecret } from '@/services/entries'
import { useLayout } from '@/hooks/useLayout'
import { keyCode } from '@/utils/keys'
import { useVisibleEntries } from '../List/useVisibleEntries'

// The list column's keyboard surface: ↑/↓ walk the visible rows, ⏎ selects the
// row they land on and ⌘⏎ copies its primary secret. On the wide shell ⇧↑/↓
// grow a batch from the anchor and ⌘A on a row picks them all. Bound to the
// column's own subtree rather than the window, so the arrows are live whether
// the search field or a row itself has focus — and dead in every other field
// the app has (the editor sheet, Settings, the palette, the generator dialog).
export const useListKeys = () => {
  const entries = useVisibleEntries()
  const currentId = useVault(state => state.currentId)
  const batching = useVault(state => state.picked.size > 0)
  // The phone picks no batches (see List/Manager).
  const batch = useLayout() === 'wide'
  // Where the keyboard sits in the list: -1 when nothing is selected, or when a
  // query has filtered the selection out from under it.
  const index = entries.findIndex(entry => entry.id === currentId)
  const order = () => entries.map(entry => entry.id)

  return (event: KeyboardEvent<HTMLElement>) => {
    // The column holds controls of its own — the title's scope menu, the sort
    // button and its menu, the group captions, the clear button — and their
    // keys bubble through here too. A
    // key is the list's only if it started in the search field or on a row, so
    // ⏎ on a button activates that button and nothing else.
    const from = event.target instanceof Element ? event.target : null
    if (!from?.closest('input[type="search"], [role="option"]')) return

    const chord = event.metaKey || event.ctrlKey
    const onRow = !!from.closest('[role="option"]')
    // In the search field ⌘A is the query's own select-all.
    if (batch && chord && onRow && keyCode(event.nativeEvent) === 'KeyA') {
      event.preventDefault()
      pickAll(order())
      return
    }

    // The row the accelerators act on: the selection while the query still
    // leaves it standing, otherwise the first row left.
    const target = entries[index] ?? entries[0]
    if (!target) return

    if (event.key === 'Enter') {
      // Left unprevented on purpose: a bare ⏎ on a row that is already open
      // belongs to the detail pane's copy accelerator (Aside/Show/Actions).
      // A batch has no one secret to copy.
      if (chord) {
        if (!batching) void copySecret(target)
      } else setCurrentEntry(target.id)
      return
    }

    const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
    if (step === 0) return
    event.preventDefault()
    // Clamped, not wrapped — the ends of the list are ends. With nothing
    // selected, either arrow opens the list at its top row.
    const next = Math.min(Math.max(index + step, 0), entries.length - 1)
    if (batch && event.shiftKey) pickRange(entries[next].id, order())
    else setCurrentEntry(entries[next].id)
  }
}
