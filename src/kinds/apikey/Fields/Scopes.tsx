import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import CopyButton from '@/components/elements/CopyButton'
import { CopyValue, Field, FieldRow, useField } from '@/components/elements/fields'
import { CHIP } from '@/components/elements/fields/chip'
import { HOVER_ONLY, ROW_COPY } from '@/components/elements/tokens'
import { scopesOf } from '../keyInfo'

// What the key may do. Typed as one line, however the issuer lists them; read
// as chips, one per scope, since that is how every issuer's console shows them.
// The chips are the row's value, so a press on them copies the list, the
// space-separated line it was typed as.
export default function Scopes() {
  const { t } = useTranslation()
  const { value, editing } = useField('scopes')

  if (editing) return <Field name="scopes" label="Scopes" placeholder="read:user repo …" />

  const scopes = scopesOf(value)
  if (scopes.length === 0) return null
  const line = scopes.join(' ')

  return (
    <FieldRow
      label="Scopes"
      actions={
        <span className={cx(HOVER_ONLY, ROW_COPY)}>
          <CopyButton value={line} title={t('Copy')} />
        </span>
      }
    >
      {() => (
        <CopyValue
          value={line}
          label={`${t('Scopes')} · ${t('Copy')}`}
          testid="entry-value-scopes"
          className="flex flex-wrap gap-1.5 py-px"
        >
          {scopes.map(scope => (
            <span key={scope} className={`${CHIP} font-mono`}>
              {scope}
            </span>
          ))}
        </CopyValue>
      )}
    </FieldRow>
  )
}
