import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CloseGlyph, PlusGlyph } from '../Main/icons'

/**
 * The one way a tag is drawn, wherever it turns up: on an entry, in the
 * editor, inside the picker's field and floating under it, under the search
 * field as the list's filter, and as the way to add one. A capsule, 24px on
 * the desktop and 32px on the phone with a hit area that reaches the
 * platform's 44px (the pseudo-element; the row around the chips gives the
 * vertical room for it). No `#` in front of it: a tag is not a hashtag, and
 * the capsule is what makes it recognisable.
 *
 * Five tones for the five places a tag can be:
 *
 * - resting, on an entry: the cards' own material — the card surface, its
 *   hairline, its white top edge and faint drop (`shadow-card`) — so a tag is
 *   a small card of the same stuff as the rows above it, lifted off the
 *   screen's ground by depth rather than by a heavy edge. Press it to see
 *   everything filed under it, or its × to take it off.
 * - field, inside the picker's field or the desktop's tags line: a flat tint
 *   with an inset hairline, the token a recipients field holds — the field
 *   under it is already the card surface, so there is nothing to lift off.
 * - active, as the one filter the list is narrowed by: the accent, solid,
 *   with the × that lifts it — the one place a tag is heavy, because it is
 *   the one thing the screen is about.
 * - outline, waiting to be picked in the sheet: the bare capsule, a hairline
 *   and the secondary ink, not yet filled in; tapped, it goes into the field.
 * - add: not a capsule at all but the quiet accent verb the screen's other
 *   "add" is (`AddAction`, "Attach file" above the tags): a plus and the
 *   words, at chip height so it sits in the row, with no box of its own — a
 *   verb beside the tags, not a tag-shaped thing that is not a tag.
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
  "relative inline-flex h-6 max-w-full flex-none cursor-pointer items-center gap-1 rounded-full px-2.5 text-xs font-medium whitespace-nowrap transition-[color,background-color,box-shadow] max-md:h-8 max-md:gap-1.5 max-md:px-3 max-md:text-base max-md:before:absolute max-md:before:inset-x-0 max-md:before:-inset-y-1.5 max-md:before:content-['']"

const TONE: Record<TagTone, string> = {
  rest: 'border border-line bg-card text-text shadow-card hover:bg-hover active:bg-sel',
  field: 'bg-tile text-text ring-1 ring-inset ring-text/8 hover:bg-text/10 active:bg-text/15',
  active: 'bg-accent text-accent-fg hover:brightness-110',
  outline: 'text-text2 ring-1 ring-inset ring-line2 hover:bg-hover hover:text-text active:bg-sel',
  add: '-mx-1.5 px-1.5 text-accent hover:bg-hover active:bg-sel max-md:-mx-2 max-md:px-2'
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
      {tone === 'add' && <PlusGlyph size={13} className="flex-none max-md:size-4" />}
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
