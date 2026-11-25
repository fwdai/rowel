import type { ReactNode } from 'react'
import type { Doc } from '../DocCell'
import Medallion from '../Medallion'
import Print from '../Print'
import type { Stock } from './stocks'

interface Props {
  stock: Stock
  doc: Doc
  /** Turns the card over; absent while it cannot be turned. */
  flip?: () => void
  /** This is the card's other side: the same stock, seen from behind. */
  back?: boolean
  children: ReactNode
}

/**
 * One side of a card: its stock's security printing, its mark in the corner —
 * the same corner on both sides, so the control that turns it stays under the
 * pointer — and on that whatever the side prints. A front is set inside the
 * card's margins; a back runs its band to the edge, so it sets its own.
 */
export default function Card({ stock, doc, flip, back, children }: Props) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <Print wash={stock.wash} line={stock.line} engraving={stock.engraving} mirrored={back} />
      <Medallion line={stock.line} onFlip={flip}>
        {stock.mark(24, doc)}
      </Medallion>
      {back ? (
        children
      ) : (
        <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">{children}</div>
      )}
    </div>
  )
}
