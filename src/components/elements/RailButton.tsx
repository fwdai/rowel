import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import Tooltip from './Tooltip'

// THE 36px tile of the 56px left rail. Resting ink is the secondary tier
// (text2) — it is the app's primary navigation and has to read as such, not as
// a row of pale hints; hover lifts it to the full ink on the hover wash.
//
// Selected is a place, not a colour: the tile becomes the rail's one raised
// key (`bg-lens` + `shadow-lens`, white on the light chrome and a lifted grey
// on the dark) with the glyph in the full ink at a heavier stroke. The lens is
// what says "you are here" in both themes, so no accent wash and no marker bar
// are needed to point at it.
//
// `action` is the rail's one verb (Add): a solid accent fill, so it reads as
// "do" and not "go". It lives here rather than in a caller's `className`
// because `cx` concatenates without merging, so a passed-in `hover:bg-*` would
// race the resting one.
export default function RailButton({
  label,
  selected,
  action,
  onClick,
  testid,
  className,
  children,
}: {
  label: string
  selected?: boolean
  action?: boolean
  onClick?: () => void
  testid?: string
  className?: string
  children: ReactNode
}) {
  return (
    <Tooltip content={label} side="right">
      <button
        type="button"
        aria-label={label}
        aria-pressed={selected}
        data-testid={testid}
        onClick={onClick}
        className={cx(
          'relative grid h-9 w-9 cursor-pointer place-items-center rounded-lg transition-[color,background-color,box-shadow]',
          action
            ? 'bg-accent text-accent-fg shadow-[inset_0_1px_0_rgba(255,255,255,0.16)] hover:bg-accent-hover'
            : selected
              ? 'bg-lens text-text shadow-lens [&_svg]:stroke-2'
              : 'text-text2 hover:bg-hover hover:text-text',
          className,
        )}
      >
        {children}
      </button>
    </Tooltip>
  )
}
