import type { CSSProperties, ReactNode } from 'react'
import Print from '../Print'
import { lineOf, type CardColor } from '../colors'

interface Props {
  color: CardColor
  /** The card's mark, for the corner of its front. */
  mark?: ReactNode
  /** This is the card's other side: the same stock, seen from behind. */
  back?: boolean
  children: ReactNode
}

/**
 * One side of a card, and everything the cards have in common: the stock's
 * printing in the card's colour, the margins, and the mark in the top-right
 * corner of the front, the place a credit card wears its network's. A document
 * type brings only what is its own — its colour by default, its mark, and how
 * its fields are arranged (see `layouts`). A back runs its band to the edge,
 * so it sets its own margins.
 *
 * The colour's ink is handed down as `--card-line`, so whatever a side draws in
 * it — the mark, a licence's category boxes — reads it rather than being told.
 */
export default function Card({ color, mark, back, children }: Props) {
  return (
    <div
      className="flex min-w-0 flex-1 flex-col"
      style={{ '--card-line': lineOf(color) } as CSSProperties}
    >
      <Print color={color} mirrored={back} />
      {back ? (
        children
      ) : (
        <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
          <div
            aria-hidden
            className="pointer-events-none absolute right-5 top-4 grid h-9 place-items-center text-(--card-line)"
          >
            {mark}
          </div>
          {children}
        </div>
      )}
    </div>
  )
}
