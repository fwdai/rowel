import type { MouseEvent } from 'react'
import { isPC } from '@/lib/platform'
import { pickRange, setCurrentEntry, togglePicked } from '@/store'

/**
 * What a click on a row does, by its modifiers: ⌘-click puts the row into the
 * batch or takes it out, ⇧-click picks the run from the anchor to it, ⌘⇧-click
 * adds that run to the batch, and a plain click opens the row alone.
 *
 * Ctrl stands in for ⌘ on a PC only: on a Mac, Ctrl-click is the secondary
 * click, never a pick.
 */
export const pickByClick = (id: string, order: readonly string[], event: MouseEvent) => {
  const toggle = isPC ? event.ctrlKey : event.metaKey
  if (event.shiftKey) pickRange(id, order, toggle)
  else if (toggle) togglePicked(id, order)
  else setCurrentEntry(id)
}
