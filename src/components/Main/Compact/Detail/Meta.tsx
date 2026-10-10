import { useTranslation } from 'react-i18next'
import { showTag } from '@/store'
import AddAction from '@/components/elements/AddAction'
import Panel from '@/components/elements/Panel'
import { ACTION_ROW, META, SECTION_LABEL } from '@/components/elements/tokens'
import { useDates } from '@/hooks/useDates'
import { toTime } from '@/utils/time'
import type { FooterProps } from '../../Body/Aside/Show/Footer'

// A tag on the phone: a pill at the row-caption size, the hairline box the
// desktop's chip has, rounded off. Pressing it goes to everything filed under
// it, the way the Tags menu does.
const TAG =
  'flex h-7 cursor-pointer items-center rounded-full border border-line2 px-3 text-sm text-text2 transition-colors active:bg-hover'

/**
 * What the entry is filed under and when it was touched, laid out for the
 * phone — the same facts `Show/Footer` draws as the desktop's strip.
 *
 * Tags are a section like the files and passkeys above them: a header over a
 * card of pills. With none, the way to add one is an action cell, the very
 * control "Attach file" is, so the two things a reader can add to an entry
 * look like the same thing. The dates are one centred meta line at the end
 * of the screen — a fact about the entry, not a value of it — rather than two
 * labelled cells that read as rows the entry had run out of room for.
 */
export default function Meta({ tags, onAdd, createdAt, updatedAt, deletedAt }: FooterProps) {
  const { t } = useTranslation()
  const { dateTime, relativeLong, shortDate } = useDates()
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

  const filed = tags.length > 0

  if (!filed && !onAdd && stamps.length === 0) return null

  return (
    <footer data-testid="entry-footer">
      {filed && (
        <div className="mt-4">
          <span className={SECTION_LABEL}>{t('Tags')}</span>
          <Panel className="flex flex-wrap gap-2 px-4 py-3">
            {tags.map(tag => (
              <button
                key={tag}
                type="button"
                onClick={() => showTag(tag)}
                aria-label={t('Filter by tag {{tag}}', { tag })}
                className={TAG}
              >
                {tag}
              </button>
            ))}
          </Panel>
        </div>
      )}
      {!filed && onAdd && (
        <AddAction
          label={t('Add tag')}
          testid="add-tag-button"
          onClick={onAdd}
          className={`mt-3 ${ACTION_ROW}`}
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
