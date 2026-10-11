import type { ReactNode } from 'react'
import { ROW_HAIRLINE } from './tokens'

interface Props {
  testid: string
  label: string
  /** The row's mark, trailing, drawn at 20px in the label's own ink. */
  glyph: ReactNode
  onClick: () => void
}

/**
 * One action in a menu, as the platform draws it: the label leading, its
 * symbol trailing in the same ink, and nothing else — no tile under the
 * symbol, no chevron, since the row does something (opens a picker, takes a
 * photo) rather than going somewhere. It is the row of the system's own
 * action menus (Files' "+", a photo's "…"), 52px tall for the thumb, with
 * the card's hairline between rows.
 *
 * `ActionRow` is the other kind: a way *in*, with a tile and a chevron,
 * because it leads to a screen or a sheet of its own.
 */
export default function MenuRow({ testid, label, glyph, onClick }: Props) {
  return (
    <div className={ROW_HAIRLINE}>
      <button
        type="button"
        data-testid={testid}
        onClick={onClick}
        className="flex h-13 w-full cursor-pointer items-center justify-between gap-4 px-4 text-left text-lg text-text transition-colors active:bg-hover hover:bg-hover"
      >
        <span className="min-w-0 flex-1 truncate">{label}</span>
        <span aria-hidden className="flex-none [&>svg]:size-5">
          {glyph}
        </span>
      </button>
    </div>
  )
}
