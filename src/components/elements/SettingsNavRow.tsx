import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ChevronRightGlyph } from '@/components/Main/icons'
import { ROW_HAIRLINE } from './tokens'

export type StatusTone = 'neutral' | 'good' | 'bad'

interface Props {
  label: ReactNode
  description?: ReactNode
  /** The row's mark, at row size (16px), in the tile a SettingsRow draws. */
  icon: ReactNode
  /** The tile lit in the accent wash: what the row leads to is switched on. */
  iconActive?: boolean
  /** Where the thing behind the row stands, said before the row is opened. */
  status?: string
  statusTone?: StatusTone
  onClick: () => void
  testid?: string
}

const DOT: Record<StatusTone, string | null> = {
  neutral: null,
  good: 'bg-good',
  bad: 'bg-bad'
}

/**
 * A row of a SettingsGroup card that leads somewhere: the whole row is the
 * button, a chevron says it opens a page, and what stands before the chevron is
 * a status rather than a control. A SettingsRow's sibling, so it keeps that
 * row's tile, type and inset, and the two can share a card.
 *
 * A grid rather than a line, because the status moves: beside the label and
 * its description on the wide shell, and onto the label's own line on the
 * phone, where a status standing beside both would squeeze the description
 * into a column a few words wide. There the two share a line that wraps, so a
 * status too long to sit beside the label (a sentence, in German) drops under
 * it rather than pushing it out of its column.
 */
export default function SettingsNavRow({
  label,
  description,
  icon,
  iconActive,
  status,
  statusTone = 'neutral',
  onClick,
  testid
}: Props) {
  const dot = DOT[statusTone]

  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      className={cx(
        'grid w-full cursor-pointer grid-cols-[auto_minmax(0,1fr)_auto_auto] items-center gap-x-3.5 px-4 py-3.5 text-left transition-colors hover:bg-hover focus-visible:bg-hover',
        ROW_HAIRLINE
      )}
    >
      <span
        className={cx(
          'col-start-1 row-span-2 row-start-1 grid h-8 w-8 place-items-center rounded-sm transition-colors',
          iconActive ? 'bg-accent-soft text-accent' : 'bg-tile text-text2'
        )}
      >
        {icon}
      </span>
      {/* One wrapping line on the phone; on the wide shell the wrapper steps
          aside (`contents`) and its two children take their own grid cells. */}
      <span className="col-start-2 row-start-1 flex flex-wrap items-baseline justify-between gap-x-3 max-md:col-end-4 md:contents">
        <span className="text-base font-medium text-text max-md:text-md md:col-start-2 md:row-start-1">
          {label}
        </span>
        {status && (
          <span
            data-testid={testid && `${testid}-status`}
            data-tone={statusTone}
            className={cx(
              'flex items-center gap-1.5 text-sm max-md:text-base md:col-start-3 md:row-span-2 md:row-start-1',
              statusTone === 'bad' ? 'text-bad' : 'text-text2'
            )}
          >
            {dot && <span aria-hidden className={cx('h-1.5 w-1.5 flex-none rounded-full', dot)} />}
            {status}
          </span>
        )}
      </span>
      {description && (
        <span className="col-start-2 row-start-2 mt-0.5 text-sm text-text2 max-md:col-end-4 max-md:text-base">
          {description}
        </span>
      )}
      <ChevronRightGlyph className="col-start-4 row-span-2 row-start-1 text-text3" />
    </button>
  )
}
