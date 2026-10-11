import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ChevronRightGlyph } from '../Main/icons'
import { ROW_HAIRLINE } from './tokens'

interface Props {
  testid: string
  /** The row's mark, at row size (16px), drawn in a 32px tile. */
  glyph: ReactNode
  label: string
  /** A second line, on desktop only: the phone's sheet has no room for it. */
  caption?: string
  /** Left out for a row that only says something (the desktop's drop hint). */
  onClick?: () => void
  /** Under the row, inside its hairline, lined up past the tile (an error). */
  children?: ReactNode
}

const ROW = 'flex w-full items-center gap-3.5 px-4 py-2.5 text-left'

/**
 * One way in, as a row of a card: the kind picker's "other ways in", the
 * phone's choice of where a photo or a file comes from (`SourceSheet`). The
 * whole row is the target — 52px tall, past the 44pt a finger needs, and on
 * the phone a 36px tile with an 18px mark makes it 56px, the grouped list's
 * row — and the chevron says it goes somewhere; the row that is only a hint
 * gets neither. The mark sits in the accent's soft tile, as a kind's does in
 * its own tint: a row here is a way in, not a setting.
 */
export default function ActionRow({ testid, glyph, label, caption, onClick, children }: Props) {
  const body = (
    <>
      <span className="grid h-8 w-8 flex-none place-items-center rounded-sm bg-accent-soft text-accent max-md:h-9 max-md:w-9 max-md:rounded-md max-md:[&>svg]:size-[18px]">
        {glyph}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-base font-medium text-text max-md:text-md">{label}</span>
        {caption && (
          <span className="mt-0.5 block text-sm text-text2 max-md:hidden">{caption}</span>
        )}
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
