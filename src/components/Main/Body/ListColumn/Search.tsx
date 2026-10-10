import { useEffect, useRef, type ReactNode } from 'react'
import { useUi, setFilterQuery } from '@/store'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { chord } from '@/lib/platform'
import Kbd from '@/components/elements/Kbd'
import SearchBox from '@/components/elements/SearchBox'
import { CloseGlyph } from '../../icons'

// The desktop field's measure: 32px under a mouse. The compact root sends the
// 44px touch one in its place (`Compact/Vault`). Size only — the surface, its
// states and their motion are SearchBox's, the same field at either size.
const DESKTOP = 'mt-3 h-8 gap-2.5 rounded-sm pl-3 pr-1.5'

// The app's one search field: it sits in the list column it filters and spans
// the column. Esc clears the query, then blurs — the accelerators that act on
// the rows (↑/↓, ⏎, ⌘⏎) belong to the whole column, so they live one level up
// in useListKeys and reach this field by bubbling.
// `className` is the box's measure (height, radius, padding), not a layout
// flag: the field is 32px where a mouse points at it and the compact root swaps
// in the 44px touch one. `filter` is a control set into the field's end, past
// the clear button — its own action, not part of the query.
export default function Search({
  className = DESKTOP,
  filter
}: {
  className?: string
  filter?: ReactNode
}) {
  const { t } = useTranslation()
  const query = useUi(state => state.query)
  const empty = query === ''
  const input = useRef<HTMLInputElement>(null)

  // ⌘F is a window-level chord, so it asks the store for the caret rather than
  // reaching into the DOM for a field it holds no ref to (`focusSearch`). The
  // request is a tick, so two presses running are two requests; the 0 it starts
  // at is the one value that is nobody asking.
  const requested = useUi(state => state.searchFocus)
  useEffect(() => {
    if (requested > 0) input.current?.focus()
  }, [requested])

  // Clearing hands the caret back to the field: the button that was clicked
  // is about to fade out and leave the tab order, and a focus left on it would
  // strand the next keystroke on an invisible control.
  const clear = () => {
    setFilterQuery('')
    input.current?.focus()
  }

  return (
    <SearchBox
      value={query}
      onChange={setFilterQuery}
      placeholder={t('Search')}
      name="search"
      testid="search-input"
      className={className}
      inputRef={input}
      trailing={
        <>
          {/* One trailing slot, both occupants stacked in its single cell so
              the hint and the clear button crossfade in place: nothing to the
              left of them moves when a query arrives or goes. */}
          <span className="grid flex-none place-items-center [&>*]:col-start-1 [&>*]:row-start-1">
          {/* The shortcut that lands here, shown while the field is idle and
              empty; focus fades it and eases it a step to the right, as if the
              caret's arrival nudged it out. A finger has no ⌘, so no coarse
              pointer ever sees it — the slot then shrinks to the clear button. */}
          <span
            aria-hidden
            className={cx(
              'transition-[opacity,transform] duration-300 any-pointer-coarse:hidden group-has-[input:focus]:translate-x-1 group-has-[input:focus]:opacity-0',
              !empty && 'translate-x-1 opacity-0'
            )}
          >
            <Kbd>{chord('F')}</Kbd>
          </span>
            {/* Always mounted so it can ease in: a query fades and grows it
                from a dot to a button, clearing does the reverse. Out of reach
                and out of the tab order while there is nothing to clear. A
                finger gets the field's full 44px to land on, around a glyph
                sized up to match. */}
            <button
              type="button"
              onClick={clear}
              aria-label={t('Clear')}
              aria-hidden={empty}
              tabIndex={empty ? -1 : 0}
              data-testid="search-clear-button"
              className={cx(
                'grid h-5 w-5 cursor-pointer place-items-center rounded-full text-text3 transition-[opacity,transform,color,background-color] hover:bg-hover hover:text-text',
                'any-pointer-coarse:h-11 any-pointer-coarse:w-11 any-pointer-coarse:[&>svg]:size-4',
                empty ? 'pointer-events-none scale-75 opacity-0' : 'scale-100 opacity-100'
              )}
            >
              <CloseGlyph size={12} />
            </button>
          </span>
          {filter}
        </>
      }
    />
  )
}
