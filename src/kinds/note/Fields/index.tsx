import { useTranslation } from 'react-i18next'
import Panel from '@/components/elements/Panel'
import SealedBlock from '@/components/elements/SealedBlock'
import { CustomFieldsField, hasExtra, useField, useFields } from '@/components/elements/fields'
import { requiredError } from '@/components/elements/fields/formats'

/**
 * A secure note, in both modes: the body is the entry's one secret (see
 * meta.ts), so it is the same object an SSH private key is — a sealed block
 * (see `elements/SealedBlock`), sealed until asked and a page to type on when
 * editing, with what it weighs in the head. The rows the user added take a
 * card of their own under it, where every other kind has them under the note.
 */
export default function Fields() {
  const { t } = useTranslation()
  const { entry, set, attempted } = useFields()
  const note = useField('note')
  // Code points, not UTF-16 units: what a person would count.
  const size = [...note.value].length

  return (
    <div className="grid gap-3">
      <SealedBlock
        label="Note"
        name="note"
        value={note.value}
        meta={t('{{size}} chars', { size })}
        lines={[note.value]}
        onChange={note.editing ? note.set : undefined}
        placeholder={t('Anything worth remembering')}
        error={note.editing ? requiredError(note.value, true, attempted) : undefined}
      />
      {(set || hasExtra(entry)) && (
        <Panel>
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
