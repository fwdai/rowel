import type { KeyboardEvent } from 'react'
import { cx } from '@/utils/cx'
import Field from '../Field'
import Note from '../Note'
import { inputClass } from '../formStyles'
import { verbatimInput } from '../inputProps'

interface Props {
  id: string
  label: string
  value: string
  disabled: boolean
  /** Shown under the field, which is painted invalid while there is one. */
  error: string | null
  testid: string
  onChange: (value: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void
}

/** One labelled password field of the restore form, with its error under it. */
export default function SecretField({
  id,
  label,
  value,
  disabled,
  error,
  testid,
  onChange,
  onKeyDown
}: Props) {
  return (
    <Field id={id} label={label}>
      <input
        id={id}
        type="password"
        {...verbatimInput}
        className={cx(inputClass, error && 'border-bad')}
        data-testid={testid}
        aria-invalid={!!error}
        value={value}
        disabled={disabled}
        onChange={event => onChange(event.target.value)}
        onKeyDown={onKeyDown}
      />
      {error && (
        <Note tone="bad" testid="form-error">
          {error}
        </Note>
      )}
    </Field>
  )
}
