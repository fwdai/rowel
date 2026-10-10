import type { ReactNode } from 'react'
import { useUi } from '@/store'
import { cx } from '@/utils/cx'
import ActiveTag from './ActiveTag'
import List from '../List'
import Search from './Search'
import Title from './Title'
import { useListKeys } from './useListKeys'
import { useListTitle } from './useListTitle'

// The middle pane: a title (the scope menu, in All Items) with the caller's
// controls and the search field, over the scrollable entry list.
interface Props {
  /**
   * The title row's controls, hidden with the rest of the filtering chrome on
   * the audit view. The wide shell sends the sort menu; the compact one, with no
   * rail to keep them on, adds the tag filter and the add button.
   */
  actions?: ReactNode
  /**
   * Replaces the 20px title block. The compact root sends the same `Title` at
   * its own 24px.
   */
  heading?: ReactNode
  /**
   * The title row's own classes — alignment and height, not side padding, which
   * the box around the row and the search gives both. Absent, the desktop's:
   * 16px under the top of the column, the controls centred on the title's
   * line — the 32px sort tile and the title's pill are then concentric, which
   * is what reads as aligned. The compact root sends its 56px nav-height row
   * (`ROOT_HEADER`).
   */
  header?: string
  /**
   * The search box's measure — height, radius, padding. Absent, it keeps the
   * 32px desktop field; the compact root sends the 44px touch one.
   */
  search?: string
  /**
   * A control set into the end of the search field, after the clear button:
   * the compact root puts its tag filter there, beside the query it narrows
   * the list with, since the phone has no rail to keep it on.
   */
  filter?: ReactNode
  /** Extra classes for the scroller, so a shell can reserve room under it. */
  scroller?: string
  /**
   * Rendered in the scroller under the listbox — outside it, so a hero and its
   * buttons are never announced as options. The compact shell has no second
   * pane, so it puts there what the wide one puts in its detail pane.
   */
  footer?: ReactNode
}

export default function ListColumn({
  actions,
  heading,
  header = 'flex items-center gap-2.5 pt-4',
  search,
  filter,
  scroller,
  footer
}: Props) {
  const view = useUi(state => state.view)
  const health = view === 'health'
  const onKeyDown = useListKeys()
  const title = useListTitle()

  return (
    <div
      // One keydown for the whole column, so ↑/↓ reach the list from the search
      // field as well as from a row. The audit list has no roving selection to
      // walk, so it is left off there.
      onKeyDown={health ? undefined : onKeyDown}
      // The column fills whatever its caller gives it: 348px of the wide shell,
      // the whole phone screen on compact.
      // The column is the wide shell's list pane, and the whole screen on the
      // phone, where it takes the shell's ground (the same 768px cut as
      // `useLayout`) so the list and the screens pushed over it share one.
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-list max-md:bg-screen"
    >
      {/* The phone keeps the header short — the list is what the screen is
          for — so the field's run-out before the first row is the row's own
          caption height, not the desktop's 10px on top of it. */}
      <div className="flex-none px-4 pb-2.5 max-md:pb-1">
        <div className={header}>
          {heading ?? <Title />}
          {/* The audit list has its own severity order — nothing to sort, and
              nothing the query below would narrow either. */}
          {!health && actions}
        </div>
        {/* The audit is not a filtered view of the vault, so the query does
            not apply to it. */}
        {!health && (
          <>
            <Search className={search} filter={filter} />
            <ActiveTag />
          </>
        )}
      </div>
      {/* The scroller is what a selected row scrolls itself into (List/Item's
          `scrollIntoView`), but it is not the listbox: the footer under the rows
          is a hero with its own buttons, and inside `role="listbox"` those are
          neither options nor reachable as controls. */}
      <div className={cx('min-h-0 flex-1 overflow-y-auto', scroller)}>
        <div role="listbox" aria-label={title}>
          <List />
        </div>
        {footer}
      </div>
    </div>
  )
}
