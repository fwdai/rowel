import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'

interface Props {
  value: string
  onChange: (value: string) => void
  // Another workspace already goes by it.
  duplicate: boolean
  disabled?: boolean
  testidPrefix: string
}

export default function NameField({ value, onChange, duplicate, disabled, testidPrefix }: Props) {
  const { t } = useTranslation()
  const id = useId()

  return (
    <Field id={id} label={t('Name')}>
      <input
        id={id}
        type="text"
        autoFocus
        maxLength={32}
        className={cx(inputClass, duplicate && 'border-bad')}
        data-testid={`${testidPrefix}-name`}
        placeholder={t('e.g. Work, Family, Clients')}
        aria-invalid={duplicate}
        value={value}
        disabled={disabled}
        onChange={event => onChange(event.target.value)}
      />
      {duplicate && (
        <Note tone="bad" testid={`${testidPrefix}-duplicate`}>
          {t('You already have a lockbox with this name.')}
        </Note>
      )}
    </Field>
  )
}
