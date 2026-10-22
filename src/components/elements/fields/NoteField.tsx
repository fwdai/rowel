import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { useField } from './context'
import { requiredError } from './formats'
import { grow } from './grow'
import FieldRow from './Row'

export default function NoteField({
  name = 'note',
  label,
  required
}: {
  name?: string
  /** Omitted for a note entry, whose body owns the whole panel. */
  label?: TKey
  required?: boolean
}) {
  const { t } = useTranslation()
  const { value, set, editing, attempted } = useField(name)
  // No label means the body is the entry (a secure note): it is written as a
  // page rather than as a row's value.
  const body = label === undefined

  if (!editing && value === '') return null

  return (
    <FieldRow label={label} error={requiredError(value, required, attempted)}>
      {id =>
        editing ? (
          <textarea
            id={id}
            name={name}
            // A note entry's body is a full-bleed row: there is no label column
            // to point at it, so it names itself.
            aria-label={body ? t('Note') : undefined}
            value={value}
            rows={1}
            placeholder={t('Anything worth remembering')}
            spellCheck={false}
            ref={grow}
            onChange={event => {
              grow(event.currentTarget)
              set(event.target.value)
            }}
            className={cx(
              'block w-full resize-none overflow-hidden bg-transparent text-base leading-relaxed text-text outline-none transition-colors placeholder:text-text2 max-md:text-md',
              // A page is tall before a word is typed, so it reads as the
              // sheet it is and never as a one-line field; the panel is its
              // box, so it draws no underline of its own. A row's note grows
              // from one line under the rule every other row's editor has.
              body
                ? 'min-h-[180px] max-md:min-h-[220px]'
                : 'min-h-6 border-b border-line2 focus:border-accent-line'
            )}
          />
        ) : (
          <div
            className="whitespace-pre-wrap break-words text-base leading-relaxed text-text2 max-md:text-md"
            data-testid={`entry-value-${name}`}
          >
            {value}
          </div>
        )
      }
    </FieldRow>
  )
}
