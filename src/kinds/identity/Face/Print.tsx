import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CARD } from './guilloche'

interface Props {
  /** The tint the stock is washed in: a CSS gradient. */
  wash: string
  /** The engraving's ink. */
  line: string
  /** The engraving itself, as one path (see `guilloche.ts`). */
  engraving: string
  /** The card's other side: the same printing, seen from behind. */
  mirrored?: boolean
  /** Anything else in the stock, as SVG in the card's own units (see `CARD`). */
  children?: ReactNode
}

/**
 * A card's security printing: the wash of colour its stock is tinted with and
 * the guilloché engraved over that, across the whole face and under everything
 * printed on the card (the face's frame is what it is pinned to, see `Frame`). It
 * is what makes a licence a licence before a word on it is read — the paper is
 * the vault's own (`Face`, in its paper tone); only the printing is the card's.
 *
 * Fixed colours, as the rest of a document face: the card stays the colour it
 * is whichever palette the app is on.
 */
export default function Print({ wash, line, engraving, mirrored, children }: Props) {
  return (
    <div
      aria-hidden
      className={cx('pointer-events-none absolute inset-0 -z-10', mirrored && '-scale-x-100')}
      style={{ background: wash }}
    >
      <svg
        viewBox={`0 0 ${CARD.width} ${CARD.height}`}
        preserveAspectRatio="xMidYMid slice"
        className="h-full w-full"
      >
        <path d={engraving} fill="none" stroke={line} strokeWidth={0.5} opacity={0.28} />
        {children}
      </svg>
    </div>
  )
}
