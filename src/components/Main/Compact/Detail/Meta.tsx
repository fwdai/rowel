import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { showTag } from '@/store'
import TagChip from '@/components/elements/TagChip'
import TagPicker from '@/components/elements/TagPicker'
import { META, SECTION_LABEL } from '@/components/elements/tokens'
import { useDates } from '@/hooks/useDates'
import { toTime } from '@/utils/time'
import type { FooterProps } from '../../Body/Aside/Show/Footer'

const same = (a: string[], b: string[]) => a.length === b.length && a.every((tag, i) => tag === b[i])

/**
 * What the entry is filed under and when it was touched, laid out for the
 * phone — the same facts `Show/Footer` draws as the desktop's strip, in both
 * faces: the read screen hands it `onSave`, the editor `onTags`.
 *
 * Tags are a section like the files above them, but not a card of anything:
 * the chips are the content, flowing from the rows' inset, with the dashed
 * "Add tag" chip at their end as the way to add one — the same shape as what
 * it adds. Pressing a tag while reading goes to everything filed under it;
 * adding or taking one off is the picker sheet, opened from the chip, which
 * never needs the editor: while reading, the sheet's choices are held here and
 * written back once, when it closes, if they changed. In the editor the sheet
 * writes the draft directly and each chip's × takes it off. The dates are one
 * centred meta line at the end of the screen — a fact about the entry, not a
 * value of it.
 */
export default function Meta({
  tags,
  onTags,
  onSave,
  subject,
  createdAt,
  updatedAt,
  deletedAt
}: FooterProps) {
  const { t } = useTranslation()
  const { dateTime, relativeLong, shortDate } = useDates()
  const [open, setOpen] = useState(false)
  // The read screen's copy of the tags while the sheet is up.
  const [pending, setPending] = useState<string[]>(tags)
  const [error, setError] = useState<string | null>(null)

  // Creation is a fact, so it reads as a date; a modification is about recency,
  // so it reads as how long ago (and as a date once that stops being useful).
  const absolute = (iso: string): string => {
    const at = toTime(iso)
    return at === null ? '' : shortDate(at)
  }
  const stamps: [string, string][] = []
  if (deletedAt) stamps.push([`${t('Deleted')} ${relativeLong(deletedAt)}`, dateTime(deletedAt)])
  if (updatedAt) stamps.push([`${t('Modified')} ${relativeLong(updatedAt)}`, dateTime(updatedAt)])
  if (createdAt) stamps.push([`${t('Created')} ${absolute(createdAt)}`, dateTime(createdAt)])

  const editing = !!onTags
  const writable = editing || !!onSave
  const filed = tags.length > 0

  if (!filed && !writable && stamps.length === 0) return null

  const pick = () => {
    setPending(tags)
    setError(null)
    setOpen(true)
  }

  const close = () => {
    setOpen(false)
    if (editing || !onSave || same(pending, tags)) return
    onSave(pending).catch(() => setError(t('Could not save. Please try again.')))
  }

  return (
    <footer data-testid="entry-footer">
      {(filed || writable) && (
        <section className="mt-4" data-testid="tags-section">
          <span className={SECTION_LABEL}>{t('Tags')}</span>
          {/* 12px between rows of chips: with the chip's 6px of hit slop above
              and below, two rows' targets meet without overlapping. */}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-3 px-4">
            {tags.map(tag =>
              onTags ? (
                <TagChip
                  key={tag}
                  removes
                  label={t('Remove tag {{tag}}', { tag })}
                  onClick={() => onTags(tags.filter(held => held !== tag))}
                >
                  {tag}
                </TagChip>
              ) : (
                <TagChip
                  key={tag}
                  label={t('Filter by tag {{tag}}', { tag })}
                  onClick={() => showTag(tag)}
                >
                  {tag}
                </TagChip>
              )
            )}
            {writable && (
              <TagChip tone="add" testid="add-tag-button" onClick={pick}>
                {t('Add tag')}
              </TagChip>
            )}
          </div>
          {error && (
            <p className="mt-2 px-4 text-base text-bad" role="alert">
              {error}
            </p>
          )}
        </section>
      )}
      {open && (
        <TagPicker
          subject={subject}
          value={editing ? tags : pending}
          onChange={onTags ?? setPending}
          onClose={close}
        />
      )}
      {stamps.length > 0 && (
        <p className={`mt-6 px-4 text-center ${META}`}>
          {stamps.map(([text, exact], index) => (
            <span key={text} title={exact}>
              {index > 0 && ' · '}
              {text}
            </span>
          ))}
        </p>
      )}
    </footer>
  )
}
