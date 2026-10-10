import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { setFilterQuery } from '@/store'
import { PlusGlyph } from '@/components/Main/icons'
import TagsInput from '@/components/elements/TagsInput'
import { TAG_CHIP } from '@/components/elements/fields/chip'
import { LABEL, META, META_TYPE } from '@/components/elements/tokens'
import { useDates } from '@/hooks/useDates'
import { toTime } from '@/utils/time'

export interface FooterProps {
  tags: string[]
  /** Editing: writes the tags back. Absent while reading. */
  onTags?: (next: string[]) => void
  /**
   * Reading an entry that can still be edited: opens the editor from the empty
   * tags cell. Absent for a tombstone, which has nothing left to file.
   */
  onAdd?: () => void
  createdAt?: string
  updatedAt?: string
  deletedAt?: string
}

type Stamp = [label: TKey, iso: string, spell: (iso: string) => string]

const stamp = (label: TKey, iso: string | undefined, spell: Stamp[2]): Stamp | null =>
  iso ? [label, iso, spell] : null

// The pane's footer, the same in both modes: what the entry is filed under, and
// when it was made and last touched. Secondary by design — a hairline away from
// the content, a small label over each value — but structured, so it never
// reads as a sentence tacked on under the rows. Tags take the left, where the
// title sits in the header; the timestamps hold the right, under the header's
// actions, and stay there whether or not there are tags. Every cell has the
// same two lines, so the labels share one baseline and the values another.
export default function Footer({
  tags,
  onTags,
  onAdd,
  createdAt,
  updatedAt,
  deletedAt
}: FooterProps) {
  const { t } = useTranslation()
  const { dateTime, relativeLong, shortDate } = useDates()
  // Creation is a fact, so it reads as a date; a modification is about recency,
  // so it reads as how long ago (and as a date once that stops being useful).
  const absolute = (iso: string): string => {
    const at = toTime(iso)
    return at === null ? '' : shortDate(at)
  }
  const stamps = [
    stamp('Deleted', deletedAt, relativeLong),
    stamp('Modified', updatedAt, relativeLong),
    stamp('Created', createdAt, absolute)
  ].filter((entry): entry is Stamp => entry !== null)
  // The cell is there whenever tags could be: once the eye has learned that
  // the left of the footer is a slot, an entry without tags would otherwise
  // read as a hole. Empty, the slot is the way to fill it. Only a tombstone,
  // which cannot be edited, has no cell.
  const filed = !!onTags || !!onAdd || tags.length > 0

  if (!filed && stamps.length === 0) return null

  return (
    <footer
      data-testid="entry-footer"
      // Narrow: a strip with two ends needs two ends to have. Below 420px of
      // container the cells stack instead, tags first, and the stamps give up
      // the right edge they were holding against the header's actions.
      // On the phone the cards above already draw the edge, so the footer is
      // a quiet block under them rather than a ruled section of its own.
      className="mt-5 flex items-start gap-10 border-t border-line pt-4 @max-[420px]:flex-col @max-[420px]:gap-4 max-md:border-0 max-md:px-1 max-md:pt-0"
    >
      {filed && (
        <Cell label="Tags" className="min-w-0 flex-1">
          {onTags ? (
            <TagsInput value={tags} onChange={onTags} placeholder={t('Add tag')} />
          ) : tags.length === 0 ? (
            // Chip-height, so the footer is the same height with or without tags.
            <button
              type="button"
              data-testid="add-tag-button"
              onClick={onAdd}
              className={`flex h-6 cursor-pointer items-center gap-1 ${META} transition-colors hover:text-text`}
            >
              <PlusGlyph size={12} />
              {t('Add tag')}
            </button>
          ) : (
            <div className="flex flex-wrap gap-2">
              {tags.map(tag => (
                <button
                  key={tag}
                  type="button"
                  onClick={() => setFilterQuery(tag)}
                  aria-label={t('Filter by tag {{tag}}', { tag })}
                  className={`${TAG_CHIP} hover:text-text`}
                >
                  {tag}
                </button>
              ))}
            </div>
          )}
        </Cell>
      )}
      {stamps.length > 0 && (
        <div className="ml-auto flex flex-none gap-10 @max-[420px]:ml-0 @max-[420px]:gap-6">
          {stamps.map(([label, iso, spell]) => (
            <Cell key={label} label={label} className="text-right @max-[420px]:text-left">
              {/* Chip-height like the tags beside it, so the value sits on the
                  same line as the chip text and the cells end together. A step
                  up from the label in ink and weight, but still short of the
                  content's full ink: within the footer the value is the
                  information and the label only says what it is a value of,
                  yet the footer as a whole stays meta to the rows above it. */}
              <span
                className={`flex h-6 items-center justify-end ${META_TYPE} font-medium text-text2 @max-[420px]:justify-start`}
                title={dateTime(iso)}
              >
                {spell(iso)}
              </span>
            </Cell>
          ))}
        </div>
      )}
    </footer>
  )
}

// One footer cell: the micro-label over the value, the way the rows pair theirs
// — only stacked, since the footer is a strip rather than a column, and flush:
// the value's own line height is all the air the pair needs. Every cell is
// built the same, so a row of them lines up twice: labels with labels, values
// with values. What sits under the label is the caller's (chips, an input, a
// date) and stays a tier down from the content above.
function Cell({
  label,
  className,
  children
}: {
  label: TKey
  className?: string
  children: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <div className={className}>
      <div className={LABEL}>{t(label)}</div>
      <div>{children}</div>
    </div>
  )
}
