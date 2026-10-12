import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CloseGlyph, PlusGlyph } from '../Main/icons'

/**
 * The one way a tag is drawn, wherever it turns up: on an entry, in the
 * editor, inside the picker's field and floating under it, under the search
 * field as the list's filter, and as the way to add one. A pill, 24px on the
 * desktop and 32px on the phone with a hit area that reaches the platform's
 * 44px (the pseudo-element; the row around the chips gives the vertical room
 * for it). No `#` in front of it: a tag is not a hashtag, and the pill is what
 * makes it recognisable.
 *
 * Five tones for the five places a tag can be:
 *
 * - resting, on an entry: the detail surface in an edge tinted with the accent
 *   — the cards' own white on the screen's grey ground, drawn a step firmer
 *   than a card's hairline so the pill holds its shape beside the content
 *   without colouring it. Press it to see everything filed under it, or its ×
 *   to take it off.
 * - field, resting inside the picker's field or the desktop's tags line: the
 *   same chip on the tile grey, since the field under it is already the
 *   detail surface and white on white would be no chip at all.
 * - active, as the one filter the list is narrowed by: the accent, solid,
 *   with the × that lifts it — the one place a tag is heavy, because it is
 *   the one thing the screen is about.
 * - outline, waiting to be picked in the sheet: the bare pill with a hairline
 *   and the secondary ink, not yet filled in; tapped, it goes into the field.
 * - add: the dashed outline of a tag not yet there — the add and create
 *   controls, the same shape so that what they add is what they look like.
 */
export type TagTone = 'rest' | 'field' | 'active' | 'outline' | 'add'

interface Props {
  tone?: TagTone
  /** A trailing ×: pressing the chip takes something away — a tag from the draft, the filter from the list. */
  removes?: boolean
  /** A quiet figure after the name: how many entries carry the tag. */
  meta?: ReactNode
  /** The accessible name, when the visible text is not the whole story ("Filter by tag work"). */
  label?: string
  title?: string
  testid?: string
  onClick: () => void
  className?: string
  children: ReactNode
}

const PILL =
  "relative inline-flex h-6 max-w-full flex-none cursor-pointer items-center gap-1 rounded-full px-2.5 text-xs font-medium whitespace-nowrap transition-colors max-md:h-8 max-md:gap-1.5 max-md:px-3 max-md:text-base max-md:before:absolute max-md:before:inset-x-0 max-md:before:-inset-y-1.5 max-md:before:content-['']"

// The resting edge: the accent at a quarter, which on the ink theme is a
// firmer grey than a card's hairline and on a coloured accent carries a trace
// of it — tinted rather than plain.
const EDGE = 'border border-accent/25'

const TONE: Record<TagTone, string> = {
  rest: `${EDGE} bg-detail text-text hover:bg-hover active:bg-sel`,
  field: `${EDGE} bg-tile text-text hover:bg-text/10 active:bg-text/15`,
  active: 'bg-accent text-accent-fg hover:brightness-110',
  outline: 'border border-line2 text-text2 hover:bg-hover hover:text-text active:bg-sel',
  add: 'border border-dashed border-line2 text-text2 hover:border-text3 hover:text-text active:bg-hover'
}

export default function TagChip({
  tone = 'rest',
  removes,
  meta,
  label,
  title,
  testid,
  onClick,
  className,
  children
}: Props) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title}
      data-testid={testid}
      onClick={onClick}
      className={cx(PILL, TONE[tone], className)}
    >
      {tone === 'add' && <PlusGlyph size={12} className="flex-none max-md:size-3.5" />}
      <span className="min-w-0 truncate">{children}</span>
      {meta !== undefined && (
        <span className="flex-none text-text3 tabular-nums" aria-hidden>
          {meta}
        </span>
      )}
      {removes && <CloseGlyph size={11} className="flex-none opacity-60 max-md:size-3" />}
    </button>
  )
}
