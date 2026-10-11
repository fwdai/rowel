import { useTranslation } from 'react-i18next'
import type { ExtraField } from '@/api/types'
import AddAction from '../../AddAction'
import Panel from '../../Panel'
import { SECTION_LABEL } from '../../tokens'
import { useFields } from '../context'
import { isBlank, rowsOf } from './extras'
import CustomFieldRow from './Row'

/**
 * Free-form label/value pairs on an entry, in both modes.
 *
 * Kind-agnostic by design: a document's shape varies past the fixed rows
 * ("Categories: B, BE"), and so does anything else a user wants to keep beside
 * an entry, so nothing here knows which kind it is rendering for. Reading, it is
 * one detail row per pair and nothing at all when there are none; editing, each
 * row is a label box and a value box that can be dropped, plus one button that
 * appends another. Most entries never have one, so until the first row is added
 * the editor shows that button alone — no heading over an empty section.
 */
export default function CustomFields({ name = 'extra' }) {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const rows = rowsOf(entry[name])
  // Reading, a blank row is not a row — only the editor has any use for one.
  const shown = set ? rows : rows.filter(field => !isBlank(field))

  if (!set && shown.length === 0) return null

  const write = (next: ExtraField[]) => set?.(name, next)
  const append = () => write([...rows, { label: '', value: '' }])

  const any = shown.length > 0

  return (
    <div className="mt-4">
      {any && (
        <>
          <span className={SECTION_LABEL}>{t('Custom fields')}</span>
          <Panel>
            {shown.map((field, index) => (
              // Position is the only identity a pair has; a label is free text
              // and two rows may share one (or have none yet).
              <CustomFieldRow
                key={index}
                field={field}
                index={index}
                onChange={
                  set
                    ? next => write(rows.map((row, i) => (i === index ? next : row)))
                    : undefined
                }
                onRemove={set ? () => write(rows.filter((_, i) => i !== index)) : undefined}
                onAppend={set && index === shown.length - 1 ? append : undefined}
              />
            ))}
          </Panel>
        </>
      )}
      {set && (
        <AddAction
          label={t('Add field')}
          testid="add-extra-field"
          onClick={append}
          className={any ? 'mt-1.5' : undefined}
        />
      )}
    </div>
  )
}
