import type { ReactNode } from 'react'
import Print from '../Print'
import { lineOf, type CardColor } from '../colors'

interface Props {
  color: CardColor
  /** The card's mark, for its top-right corner. */
  mark: ReactNode
  children: ReactNode
}

/**
 * A card, and everything the cards have in common: the stock's printing in the
 * card's colour, the margins, and the mark in the top-right corner, the place
 * a credit card wears its network's. A document type brings only what is its
 * own — its colour by default, its mark, and how its fields are arranged (see
 * `layouts`).
 */
export default function Card({ color, mark, children }: Props) {
  return (
    <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
      <Print color={color} />
      <div
        aria-hidden
        className="pointer-events-none absolute right-5 top-4 grid h-9 place-items-center"
        style={{ color: lineOf(color) }}
      >
        {mark}
      </div>
      {children}
    </div>
  )
}
