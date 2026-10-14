import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { ChevronRightGlyph } from '@/components/Main/icons'
import { META } from '@/components/elements/tokens'

interface Props {
  /** The provider's mark, or a glyph for a source that has none. */
  mark: ReactNode
  title: string
  body: string
  /**
   * A glyph chip rather than a plate: for a source with no brand mark of its
   * own (a cloud, a backup file). Tinted with the accent so it belongs to the
   * app rather than to a logo, and so it follows the chosen accent and theme.
   */
  muted?: boolean
  onClick: () => void
  testid?: string
}

// The glyph chip. A soft wash of the accent, lit from the top-left like the
// cards around it, with the glyph inked in the accent itself: a tinted tile,
// not a grey one. It answers the row's hover by deepening a shade — a fill
// under the wash at less than half the tint's strength, eased in — so the
// chip moves with the row instead of sitting on it, without flashing solid.
const GLYPH_CHIP = cx(
  'text-accent',
  'bg-[linear-gradient(135deg,var(--c-accent-soft),transparent_80%)]',
  'ring-1 ring-inset ring-accent-soft',
  'shadow-[inset_0_1px_0_var(--topglow)]',
  'transition-[background-color] duration-300',
  'group-hover:bg-[color:color-mix(in_srgb,var(--c-accent-soft)_40%,transparent)]'
)

// The plate: a provider's own mark in its own colours, on a neutral ground
// that lets the brand read as itself.
const MARK_PLATE = 'bg-detail ring-1 ring-line2 shadow-[0_1px_2px_rgba(20,22,26,0.06)]'

// One way back into your own data, as a row you press: the provider's mark on
// a plate, what pressing it does, a meta line on what that means, and a
// chevron that leans into the press. Shorter than the "Start fresh" button
// above it — the title is the row's one line of real type — so the two rows
// together weigh about what that one button does, and it moves under the
// pointer (border, lift, chevron) so it never reads as a static card.
export default function RestoreAction({ mark, title, body, muted, onClick, testid }: Props) {
  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      className={cx(
        'group flex w-full cursor-pointer items-center gap-3 rounded-lg border border-line bg-card p-2 pr-3 text-left',
        'shadow-[0_1px_2px_rgba(20,22,26,0.05),inset_0_1px_0_var(--topglow)]',
        // The slow tier for the hover lift so it is seen moving, and a quick
        // snap on press so the row answers the finger rather than trailing it.
        'transition-[border-color,box-shadow,transform] duration-300 hover:border-accent-line hover:shadow-[0_8px_22px_rgba(20,22,26,0.09),inset_0_1px_0_var(--topglow)] active:scale-[0.985] active:shadow-[0_1px_2px_rgba(20,22,26,0.05),inset_0_1px_0_var(--topglow)] active:duration-100'
      )}
    >
      <span
        className={cx(
          'grid h-9 w-9 flex-none place-items-center rounded-[10px]',
          muted ? GLYPH_CHIP : MARK_PLATE
        )}
      >
        {mark}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-base font-medium text-text">{title}</span>
        <span className={`block ${META}`}>{body}</span>
      </span>
      <ChevronRightGlyph className="flex-none text-text3 transition-[color,transform] duration-300 group-hover:translate-x-0.5 group-hover:text-accent" />
    </button>
  )
}
