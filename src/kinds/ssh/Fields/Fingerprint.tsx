import { useTranslation } from 'react-i18next'
import CopyButton from '@/components/elements/CopyButton'
import { CopyValue, FieldRow, useField } from '@/components/elements/fields'
import { ROW_COPY, VALUE } from '@/components/elements/tokens'

// Derived, never typed: the generator stamps it with the key, so the editor's
// row is read-only, and a key pasted in by hand simply has none to show. The
// read view sets it on the plate instead (see `Face`).
export default function Fingerprint() {
  const { t } = useTranslation()
  const { value } = useField('fingerprint')

  if (value === '') return null

  return (
    <FieldRow
      label="Fingerprint"
      actions={
        <span className={ROW_COPY}>
          <CopyButton value={value} title={t('Copy')} />
        </span>
      }
    >
      {() => (
        <CopyValue
          value={value}
          label={`${t('Fingerprint')} · ${t('Copy')}`}
          testid="entry-value-fingerprint"
          className={`${VALUE} text-text`}
        >
          {value}
        </CopyValue>
      )}
    </FieldRow>
  )
}
