import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError, errorKind, isTooManyAttempts } from '@/api/errors'
import { syncAdoptPassword } from '@/api/sync'
import { closePasswordPrompt } from '@/store'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { useRetryAfter } from './useRetryAfter'

/**
 * The new password, checked by the backend against the account's pack. The
 * field is emptied as soon as it is sent, whatever the answer: a wrong one is
 * retyped, a right one has no business staying on screen. A wrong password
 * counts against the same backoff as a wrong unlock, so the countdown is the
 * lock screen's.
 */
export default function AdoptForm({ onAdopted }: { onAdopted: (unmoved: string[]) => void }) {
  const { t } = useTranslation()
  const id = useId()
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [retryAfter, setRetryAfter] = useRetryAfter()
  const locked = retryAfter > 0

  const message = locked
    ? t('Too many failed attempts. Try again in {{seconds}}s', { seconds: retryAfter })
    : error

  const submit = () => {
    if (!password || busy || locked) return
    const typed = password
    setPassword('')
    setBusy(true)
    setError(null)
    syncAdoptPassword(typed)
      .then(onAdopted)
      .catch((err: unknown) => {
        if (isTooManyAttempts(err)) setRetryAfter(err.retryAfterSecs)
        else if (errorKind(err) === 'invalidPassword')
          setError(
            t(
              "That isn't the new password. If you didn't change it on another device, the account's copy may not be yours; you can replace it from Settings › Lockboxes."
            )
          )
        else setError(describeError(err) || t('Something went wrong'))
      })
      .finally(() => setBusy(false))
  }

  return (
    <>
      <div className="mt-4">
        <Field id={id} label={t('New password')}>
          <input
            id={id}
            type="password"
            name="adopt_password"
            {...verbatimInput}
            data-testid="password-changed-password"
            className={cx(inputClass, message && 'border-bad')}
            aria-invalid={!!message}
            value={password}
            disabled={busy || locked}
            onChange={event => {
              setError(null)
              setPassword(event.target.value)
            }}
            onKeyDown={event => event.key === 'Enter' && submit()}
          />
          {message && (
            <Note tone="bad" testid="password-changed-error">
              {message}
            </Note>
          )}
        </Field>
      </div>

      <div className="mt-5 flex items-center gap-2">
        <Button
          size="md"
          onClick={submit}
          disabled={!password || locked}
          loading={busy}
          testid="password-changed-submit"
        >
          {t('Use new password')}
        </Button>
        <Button
          variant="pale"
          size="md"
          onClick={closePasswordPrompt}
          testid="password-changed-cancel"
        >
          {t('Not now')}
        </Button>
      </div>
    </>
  )
}
