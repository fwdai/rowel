import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CheckGlyph } from '@/components/Main/icons'

interface Props {
  /** Up for the beat `useCopied` holds after a copy. */
  copied: boolean
  /**
   * The check's ink: the `good` verdict on a panel row, and on a face the
   * face's own ink (`text-(--face-ink)`), so the mark reads as printed on the
   * object — the pane's green has no business on black plastic or on paper.
   */
  ink?: string
  /** The value as it shows at rest. */
  children: ReactNode
}

/**
 * How a copy says it landed, wherever the value sits — a detail row, a chip
 * list, a cell on a document, the number on a card: the value's glyphs go
 * invisible and one check takes their place for the beat, then they come back.
 *
 * Invisible rather than gone, so the row keeps its size; a glyph rather than a
 * word, so it fits the narrowest cell and the control's text — what a testid
 * is read for — is still the value alone; sized in em, so it is a step bigger
 * than the type it stands in for whatever the line's tier; a mark in the ink
 * of the surface and never a block painted over it, so there is nothing to get
 * wrong against a dark card or a dark theme. The caller is `relative`, so the
 * check has something to sit in. `contents`, so a chip row's chips are still
 * the button's flex items while they are hidden.
 */
export default function Copied({ copied, ink = 'text-good', children }: Props) {
  if (!copied) return <>{children}</>

  return (
    <>
      <span aria-hidden className="invisible contents">
        {children}
      </span>
      <span
        data-testid="copied-mark"
        className={cx('pointer-events-none absolute inset-y-0 left-0 flex items-center', ink)}
      >
        <CheckGlyph className="size-[1.2em]" />
      </span>
    </>
  )
}
