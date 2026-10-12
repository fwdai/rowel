import { forwardRef, type ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CARD, META, META_TYPE } from '@/components/elements/tokens'

interface Props {
  badge: ReactNode
  name: string
  hint: string
  active: boolean
  disabled: boolean
  onClick: () => void
  testid: string
  // What the tile opens: a menu, for the one that holds the other apps.
  haspopup?: 'menu'
  expanded?: boolean
}

// One card in the import grid: a badge, the app's name and what file it
// exports. The same shape whether it opens the file picker or a menu.
const Tile = forwardRef<HTMLButtonElement, Props>(function Tile(
  { badge, name, hint, active, disabled, onClick, testid, haspopup, expanded },
  ref
) {
  return (
    <button
      ref={ref}
      type="button"
      disabled={disabled}
      data-testid={testid}
      aria-haspopup={haspopup}
      aria-expanded={expanded}
      onClick={onClick}
      className={cx(
        CARD,
        'flex w-full cursor-pointer items-center gap-3 p-3 text-left transition-colors hover:border-accent-line',
        active && 'border-accent-line',
        disabled && 'cursor-default opacity-50'
      )}
    >
      <div
        className={`grid h-[34px] w-[34px] flex-none place-items-center rounded-sm bg-tile ${META_TYPE} font-semibold text-text2`}
      >
        {badge}
      </div>
      <div className="min-w-0">
        <div className="truncate text-base font-medium text-text">{name}</div>
        <div className={`mt-0.5 ${META}`}>{hint}</div>
      </div>
    </button>
  )
})

export default Tile
