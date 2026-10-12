import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { ExtraField } from '@/api/types'
import { cx } from '@/utils/cx'
import AddAction from '../../AddAction'
import { ACTION_ROW, BAND_GUTTER } from '../../tokens'
import { useFields } from '../context'
import { isBlank, rowsOf } from './extras'
import CustomFieldRow from './Row'

/**
 * Free-form label/value pairs on an entry, in both modes, as the last band of
 * the card the fixed fields are on: the same card and the same row geometry,
 * behind a gutter cut through the card, so the user's rows read as the
 * entry's fields and still as the ones the user added — a grouped list's
 * second group, without a heading to announce it. The user's label is each
 * row's caption, as it is for the fixed rows.
 *
 * Kind-agnostic by design: a document's shape varies past the fixed rows
 * ("Categories: B, BE"), and so does anything else a user wants to keep beside
 * an entry, so nothing here knows which kind it is rendering for. Reading, it is
 * one detail row per pair and nothing at all when there are none (a kind whose
 * read face has no card of rows asks `hasExtra` before drawing one for these);
 * editing, each row is a label box and a value box that can be dropped, and
 * the band ends on the quiet row that appends another.
 */
export default function CustomFields({ name = 'extra' }) {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const rows = rowsOf(entry[name])
  // Reading, a blank row is not a row — only the editor has any use for one.
  const shown = set ? rows : rows.filter(field => !isBlank(field))
  // The row the last "Add field" made, which takes the caret as it mounts.
  // Rows are keyed by position, so only a row mounting at this index — the
  // appended one — ever reads it; the ones already on screen stay put.
  const [added, setAdded] = useState<number | null>(null)

  if (!set && shown.length === 0) return null

  const write = (next: ExtraField[]) => set?.(name, next)
  const append = () => {
    write([...rows, { label: '', value: '' }])
    setAdded(rows.length)
  }

  return (
    <>
      <div data-gutter className={BAND_GUTTER} />
      {/* A band of its own, so the hairline stops at its last row. */}
      <div>
        {shown.map((field, index) => (
          // Position is the only identity a pair has; a label is free text and
          // two rows may share one (or have none yet).
          <CustomFieldRow
            key={index}
            field={field}
            index={index}
            onChange={
              set ? next => write(rows.map((row, i) => (i === index ? next : row))) : undefined
            }
            onRemove={set ? () => write(rows.filter((_, i) => i !== index)) : undefined}
            onAppend={set && index === shown.length - 1 ? append : undefined}
            autoFocus={index === added}
          />
        ))}
        {set && (
          // The band's last row, at the fixed rows' inset: the accent text
          // line at the value line's height on the desktop, and on the phone
          // the same dashed pill as every other "add" on the screen.
          <div className="px-3.5 py-3 max-md:px-4 max-md:py-2">
            <AddAction
              label={t('Add field')}
              testid="add-extra-field"
              onClick={append}
              className={cx('h-6', ACTION_ROW)}
            />
          </div>
        )}
      </div>
    </>
  )
}
