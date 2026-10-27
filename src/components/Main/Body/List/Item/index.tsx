import { useEffect, useRef, type MouseEvent } from 'react'
import { cx } from '@/utils/cx'
import { useVault, selectBatch, setCurrentEntry } from '@/store'
import type { EntryMeta } from '@/api/types'
import { kindOf } from '@/kinds'
import { useDates } from '@/hooks/useDates'
import { useTranslation } from 'react-i18next'
import { StarGlyph } from '../../../icons'
import { stampOf } from '../order'
import Flag from './Flag'
import { flagOf } from './audit'
import { META } from '@/components/elements/tokens'

interface Props {
  entry: EntryMeta
  /**
   * What a click does, modifiers and all (see List/pick). Absent, it opens the
   * row: the audit list and the phone pick no batches.
   */
  onClick?: (event: MouseEvent) => void
}

export default function Item({ entry, onClick }: Props) {
  const { t } = useTranslation()
  const { relativeTime } = useDates()
  const ref = useRef<HTMLDivElement>(null)
  // Where the keyboard sits — the open row, or a batch's last-clicked one.
  const current = useVault(state => state.currentId === entry.id)
  // In a batch every picked row wears the selection; otherwise the open one.
  const selected = useVault(state =>
    selectBatch(state) ? state.picked.has(entry.id) : state.currentId === entry.id
  )
  // Read the flag off the audit the vault already ran on unlock — never score
  // a password during a row render (see src/hooks/useStrength.ts).
  const flagKind = useVault(state => flagOf(state.audit?.[entry.id]))

  const flag = flagKind ? <Flag kind={flagKind} /> : undefined
  // A tombstone's one useful stamp is when it went, and it needs the word: "3d"
  // alone would read as "edited 3 days ago" exactly like every live row.
  const meta = entry.deletedAt
    ? t('Deleted {{time}}', { time: relativeTime(entry.deletedAt) })
    : relativeTime(stampOf(entry))
  const Content = kindOf(entry.type).ListRow

  // Keep the selected row in view when the keyboard walks past the fold, and
  // carry focus along only when the list already had it: arrowing from the
  // search field must leave the caret in the field.
  useEffect(() => {
    if (!current) return
    ref.current?.scrollIntoView({ block: 'nearest' })
    // Still an option at this point even when the arrows just moved off it:
    // dropping a focused row to tabIndex -1 does not blur it, so the row being
    // left keeps focus until this one takes it.
    if (document.activeElement?.getAttribute('role') === 'option') ref.current?.focus()
  }, [current])

  return (
    <div
      ref={ref}
      role="option"
      aria-selected={selected}
      // One tab stop for the whole list: Tab lands on the selection, and the
      // arrows take over from there.
      tabIndex={current ? 0 : -1}
      className={cx(
        // A finger needs a 44pt row where a mouse is happy with 30. This is a
        // capability rule, not a shell one: it holds wherever *any* pointer is
        // coarse — a touch laptop has a fine one too — and costs no render
        // branch.
        'flex cursor-pointer items-center gap-3 border-l-2 py-2.5 pl-[14px] pr-4 any-pointer-coarse:py-3.5',
        // The rule under the row: full-width in the desktop's list pane, and
        // on the phone a rule from the text column to the edge, the way iOS
        // separates rows — 16px gutter + 40px tile + the 14px gap it takes.
        'inset-shadow-hairline max-md:gap-3.5 max-md:inset-shadow-none max-md:hairline-inset-[70px]',
        selected
          ? 'border-accent bg-sel'
          : 'border-transparent hover:bg-hover'
      )}
      data-testid="entry-item"
      onClick={onClick ?? (() => setCurrentEntry(entry.id))}
    >
      <Content entry={entry} flag={flag} />
      {entry.favorite && (
        <span data-testid="entry-item-star" className="flex-none text-accent">
          <StarGlyph size={12} filled />
        </span>
      )}
      {meta && (
        <span className={`flex-none ${META}`}>{meta}</span>
      )}
    </div>
  )
}
