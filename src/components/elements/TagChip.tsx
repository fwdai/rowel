import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CloseGlyph, PlusGlyph } from '../Main/icons'

/**
 * The one way a tag is drawn, wherever it turns up: on an entry, in the
 * editor, under the search field as the list's filter, and as the way to add
 * one. A pill with a soft fill and no edge of its own — nothing else in the
 * app is that shape on that surface, which is what makes a tag recognisable
 * as one without a `#` in front of it. 24px on the desktop, 32px on the phone
 * with a hit area that reaches the platform's 44px (the pseudo-element; the
 * row around the chips gives the vertical room for it).
 *
 * Three tones for the three things a tag can be doing: resting on an entry
 * (press it to see everything filed under it), standing as the one filter the
 * list is narrowed by (the accent, with the × that lifts it), and the dashed
 * outline of a tag not yet there — the add control, which is the same shape so
 * that the thing it adds is what it looks like.
 */
export type TagTone = 'rest' | 'active' | 'add'

interface Props {
  tone?: TagTone
  /** A trailing ×: pressing the chip takes something away — a tag from the draft, the filter from the list. */
  removes?: boolean
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

const TONE: Record<TagTone, string> = {
  rest: 'bg-tile text-text hover:bg-text/10 active:bg-text/15',
  active: 'bg-accent text-accent-fg hover:brightness-110',
  add: 'border border-dashed border-line2 text-text2 hover:border-text3 hover:text-text active:bg-hover'
}

export default function TagChip({
  tone = 'rest',
  removes,
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
      {removes && <CloseGlyph size={11} className="flex-none opacity-60 max-md:size-3" />}
    </button>
  )
}
