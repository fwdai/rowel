import { useId, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { changeMasterPassword } from '@/api/auth'
import { describeError } from '@/api/errors'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import IconButton from '@/components/elements/IconButton'
import Note from '@/components/elements/Note'
import PasswordStrength from '@/components/elements/PasswordStrength'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { EyeGlyph, EyeOffGlyph, KeyGlyph } from '@/components/Main/icons'
import ExpandableRow from '../ExpandableRow'

type Name = 'current_password' | 'new_password' | 'new_password_repeat'

export default function MasterPasswordRow() {
  const { t } = useTranslation()
  const id = useId()
  const [values, setValues] = useState<Partial<Record<Name, string>>>({})
  const [shown, setShown] = useState(false)
  const [processing, setProcessing] = useState(false)
  const [success, setSuccess] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const current = values.current_password ?? ''
  const next = values.new_password ?? ''
  const repeat = values.new_password_repeat ?? ''
  const mismatch = !!repeat && next !== repeat
  const disabled = !current || !next || !repeat || mismatch

  const input = (name: Name, type = 'password', className?: string) => (
    <input
      id={`${id}-${name}`}
      type={type}
      name={name}
      {...verbatimInput}
      className={cx(inputClass, className)}
      value={values[name] ?? ''}
      onChange={e => setValues({ ...values, [name]: e.target.value })}
    />
  )

  const field = (name: Name, label: TKey, control: ReactNode) => (
    <Field id={`${id}-${name}`} label={t(label)}>
      {control}
    </Field>
  )

  const onSubmit = () => {
    if (disabled) return
    setProcessing(true)
    setError(null)
    setSuccess(null)
    changeMasterPassword(current, next)
      .then(unchanged => {
        setValues({})
        // The change reaches every workspace that shares the password, so the
        // ones it did not reach are the news: a workspace with a password of
        // its own keeps it, and so does one whose own re-key failed.
        setSuccess(
          unchanged?.length
            ? t('Password changed. Some lockboxes on this device kept their own password.')
            : t('Password changed')
        )
      })
      .catch((err: unknown) => setError(describeError(err)))
      .finally(() => setProcessing(false))
  }

  return (
    <ExpandableRow
      label={t('Master password')}
      description={t(
        'Every lockbox on this device that shares this password changes with it'
      )}
      icon={<KeyGlyph size={16} />}
      action={t('Change…')}
      testid="settings-master-password-row"
    >
      <div className="flex max-w-sm flex-col gap-3">
        {field('current_password', 'Current password', input('current_password'))}
        {field(
          'new_password',
          'New password',
          <>
            <div className="relative">
              {input('new_password', shown ? 'text' : 'password', 'pr-10')}
              <div className="absolute top-1/2 right-1 -translate-y-1/2">
                <IconButton
                  label={shown ? t('Hide') : t('Show')}
                  active={shown}
                  onClick={() => setShown(!shown)}
                >
                  {shown ? <EyeOffGlyph /> : <EyeGlyph />}
                </IconButton>
              </div>
            </div>
            <PasswordStrength password={next} />
          </>
        )}
        {field(
          'new_password_repeat',
          'Repeat new password',
          <>
            {input('new_password_repeat')}
            {mismatch && <Note tone="bad">{t("Passwords don't match yet.")}</Note>}
          </>
        )}
        <div className="flex flex-wrap items-center gap-3">
          <Button
            testid="change-password-submit"
            onClick={onSubmit}
            disabled={disabled}
            loading={processing}
          >
            {t('Update')}
          </Button>
          {error && (
            <Note tone="bad" testid="change-password-error">
              {error}
            </Note>
          )}
          {success && (
            <Note tone="good" testid="change-password-success">
              {success}
            </Note>
          )}
        </div>
      </div>
    </ExpandableRow>
  )
}
