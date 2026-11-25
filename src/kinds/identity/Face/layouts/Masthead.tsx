import type { ReactNode } from 'react'

interface Props {
  /** What leads the row: the issuing state's plate, or its code. */
  lead?: ReactNode
  title?: ReactNode
  /** The quieter line under the title, or the only line where there is no title. */
  sub?: ReactNode
}

/**
 * A card's masthead: what it is and whose, across its head — cleared on the
 * right for the mark in the corner (see `Card`).
 */
export default function Masthead({ lead, title, sub }: Props) {
  return (
    <header className="flex h-9 flex-none items-center gap-3 pr-12">
      {lead}
      <div className="min-w-0 flex-1">
        {title && (
          <div className="truncate text-base font-semibold uppercase leading-tight tracking-label">
            {title}
          </div>
        )}
        {sub && (
          <div className="truncate text-2xs uppercase tracking-[0.1em] text-(--face-ink2)">{sub}</div>
        )}
      </div>
    </header>
  )
}
