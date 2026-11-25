import type { ReactNode } from 'react'
import type { Doc } from '../DocCell'
import Print from '../Print'
import type { Stock } from './stocks'

interface Props {
  stock: Stock
  doc: Doc
  /** This is the card's other side: the same stock, seen from behind. */
  back?: boolean
  children: ReactNode
}

/**
 * One side of a card: its stock's printing and on that whatever the side
 * prints. A front is set inside the card's margins and wears the card's mark
 * in its top-right corner, the place a credit card wears its network's; a back
 * runs its band to the edge, so it sets its own margins.
 */
export default function Card({ stock, doc, back, children }: Props) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <Print wash={stock.wash} line={stock.line} mirrored={back} />
      {back ? (
        children
      ) : (
        <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
          <div
            aria-hidden
            className="pointer-events-none absolute right-5 top-4 grid h-9 place-items-center"
            style={{ color: stock.line }}
          >
            {stock.mark(28, doc)}
          </div>
          {children}
        </div>
      )}
    </div>
  )
}
