import type { CSSProperties, ReactNode } from 'react'
import { hueOf } from '@/lib/hue'
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
 * Four tones for the four things a tag can be doing:
 *
 * - resting, on an entry or in the picker's field: the detail surface with a
 *   hairline edge — the cards' own materials, so the chip belongs to the
 *   screen rather than sitting on it as a grey tile — and a small dot in a
 *   hue hashed from the tag's name (`lib/hue`, `tag-dot`), so a tag has a
 *   colour that is its own and wears it the same on every entry, the way a
 *   workspace tile does, without a row of tags becoming a row of colours: the
 *   pill stays quiet and the dot is the label. Press it to see everything
 *   filed under it, or its × to take it off.
 * - active, as the one filter the list is narrowed by: the accent, solid,
 *   with the × that lifts it — the one place a tag is heavy, because it is
 *   the one thing the screen is about.
 * - outline, waiting to be picked in the sheet: the bare pill with a hairline
 *   and the secondary ink, no dot yet; tapped, it goes into the field and
 *   takes its dot and its full ink.
 * - add: the dashed outline of a tag not yet there — the add and create
 *   controls, the same shape so that what they add is what they look like.
 */
export type TagTone = 'rest' | 'active' | 'outline' | 'add'

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
  /** The tag's name — what is shown, and what the resting tint is keyed on. */
  children: string
}

const PILL =
  "relative inline-flex h-6 max-w-full flex-none cursor-pointer items-center gap-1 rounded-full px-2.5 text-xs font-medium whitespace-nowrap transition-[color,background-color,border-color,filter] max-md:h-8 max-md:gap-1.5 max-md:px-3 max-md:text-base max-md:before:absolute max-md:before:inset-x-0 max-md:before:-inset-y-1.5 max-md:before:content-['']"

const TONE: Record<TagTone, string> = {
  rest: 'border border-line bg-detail text-text hover:bg-hover active:bg-sel',
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
      style={tone === 'rest' ? ({ '--hue': hueOf(children) } as CSSProperties) : undefined}
    >
      {tone === 'rest' && (
        <span aria-hidden className="tag-dot size-1.5 flex-none rounded-full max-md:size-2" />
      )}
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
