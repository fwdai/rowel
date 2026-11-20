import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import CopyButton from '../CopyButton'
import { useField } from './context'
import { requiredError } from './formats'
import { grow } from './grow'
import FieldRow from './Row'

// The note row every kind carries under its fields, in both modes: a few lines
// of prose that grow as they are typed. Read, it is prose rather than a value
// to press: its text can be selected, and the copy button in the rail takes the
// whole. A body that is the whole entry (a secure note) is not a row but a
// sealed block (see `elements/SealedBlock`).
export default function NoteField({
  name = 'note',
  label = 'Note',
  required
}: {
  name?: string
  label?: TKey
  required?: boolean
}) {
  const { t } = useTranslation()
  const { value, set, editing, attempted } = useField(name)

  if (!editing && value === '') return null

  return (
    <FieldRow
      label={label}
      error={requiredError(value, required, attempted)}
      actions={
        !editing && <CopyButton value={value} title={t('Copy')} testid={`copy-${name}`} />
      }
    >
      {id =>
        editing ? (
          <textarea
            id={id}
            name={name}
            value={value}
            rows={1}
            placeholder={t('Anything worth remembering')}
            spellCheck={false}
            ref={grow}
            onChange={event => {
              grow(event.currentTarget)
              set(event.target.value)
            }}
            className="block min-h-6 w-full resize-none overflow-hidden border-b border-line2 bg-transparent text-base leading-relaxed text-text outline-none transition-colors placeholder:text-text2 focus:border-accent-line max-md:text-md"
          />
        ) : (
          // At least the box's one line tall, so the row's label sits where it
          // does over the editor.
          <div
            className="min-h-6 whitespace-pre-wrap break-words text-base leading-relaxed text-text2 select-text max-md:text-md"
            data-testid={`entry-value-${name}`}
          >
            {value}
          </div>
        )
      }
    </FieldRow>
  )
}
