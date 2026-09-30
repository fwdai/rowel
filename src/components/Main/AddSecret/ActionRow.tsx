import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ROW_HAIRLINE } from '@/components/elements/tokens'
import { ChevronRightGlyph } from '../icons'

interface Props {
  testid: string
  /** The row's mark, at row size (16px), drawn in a 32px tile. */
  glyph: ReactNode
  label: string
  caption: string
  /** Left out for a row that only says something (the desktop's drop hint). */
  onClick?: () => void
  /** Under the row, inside its hairline, lined up past the tile (an error). */
  children?: ReactNode
}

const ROW = 'flex w-full items-center gap-3.5 px-4 py-2.5 text-left'

/**
 * One of the picker's other ways in: a row of the card under the kind tiles.
 * The whole row is the target, and the chevron says it goes somewhere; the
 * row that is only a hint gets neither.
 */
export default function ActionRow({ testid, glyph, label, caption, onClick, children }: Props) {
  const body = (
    <>
      <span className="grid h-8 w-8 flex-none place-items-center rounded-sm bg-tile text-text2">
        {glyph}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-base font-medium text-text">{label}</span>
        <span className="mt-0.5 block text-sm text-text2">{caption}</span>
      </span>
    </>
  )

  return (
    <div className={ROW_HAIRLINE}>
      {onClick ? (
        <button
          type="button"
          data-testid={testid}
          onClick={onClick}
          className={cx(ROW, 'cursor-pointer transition-colors hover:bg-hover')}
        >
          {body}
          <ChevronRightGlyph className="flex-none text-text3" />
        </button>
      ) : (
        <div data-testid={testid} className={ROW}>
          {body}
        </div>
      )}
      {children && <div className="pr-4 pb-2.5 pl-[62px]">{children}</div>}
    </div>
  )
}
