import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ChevronRightGlyph } from '../../icons'

interface Props {
  label: string
  testid: string
  onClick: () => void
  /** The row's mark, at row size (16px), drawn in a 28px tile like the desktop nav's. */
  glyph?: ReactNode
  /** The disclosure mark: this row pushes a screen rather than doing a thing. */
  chevron?: boolean
  /** The row's ink, for the one row that is a warning (Lock vault). */
  ink?: string
}

/**
 * A tappable row of a settings card, the way an iOS settings screen draws
 * one: a tile, the name at the phone's row size, a chevron, and the rule
 * under it starting where the text does rather than framing the tile.
 *
 * Not `elements/SettingsRow`: that one is a label/control pair with no notion
 * of the whole row being the target. These rows *are* the target, so they are
 * 48px and the button is the row itself.
 */
export default function Row({
  label,
  testid,
  onClick,
  glyph,
  chevron,
  ink = 'text-text'
}: Props) {
  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      className={cx(
        // 16px inset + 28px tile + 12px gap: the rule starts under the label.
        'flex h-12 w-full cursor-pointer items-center gap-3 px-4 text-left text-md transition-colors active:bg-hover hairline-inset-14 last:after:hidden',
        ink
      )}
    >
      {glyph && (
        <span className="grid h-7 w-7 flex-none place-items-center rounded-sm bg-tile">
          {glyph}
        </span>
      )}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {chevron && <ChevronRightGlyph className="flex-none text-text3" />}
    </button>
  )
}
