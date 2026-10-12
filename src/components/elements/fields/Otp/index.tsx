import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { useOtp } from '@/hooks/useOtp'
import { META, VALUE_LINE } from '../../tokens'
import { useField } from '../context'
import CopyValue from '../CopyValue'
import Field from '../Field'
import FieldRow from '../Row'
import Countdown from './Countdown'
import { groupDigits, otpSecret, otpStored } from './secret'

/**
 * The one-time code, in both modes, as a row under the password — the way
 * Passwords and 1Password list a verification code with the credentials
 * rather than in a widget of its own.
 *
 * Reading, the current code is the value, with how long it has left as a ring
 * in the rail; the value is the copy control, as every row's is. Editing, the
 * row is the secret's own box, like every other field's, and once the secret
 * parses the live code sits under it as the one proof that what was pasted
 * actually works. The preview is not copyable: while typing, the code is
 * evidence, not a thing to take.
 */
export default function OtpField({
  name = 'otp',
  label = 'One-time code'
}: {
  name?: string
  label?: TKey
}) {
  const { t } = useTranslation()
  const { value, editing } = useField(name)
  // `otpSecret` answers "is this a secret at all"; `stored` is the thing worth
  // keeping — the same seed, plus the parameters when the link carries any the
  // generator would otherwise have to guess at. The backend reads both forms.
  const stored = otpStored(value)
  const { code, time, period } = useOtp(stored)

  if (editing)
    return (
      <Field
        name={name}
        label={label}
        placeholder={t('Paste secret or link')}
        // A pasted otpauth:// link collapses to the secret it carries —
        // unless it also carries a digit count, period or algorithm of its
        // own, which the link is the only place to keep.
        normalize={typed => otpStored(typed) || typed.trim()}
        check={typed => (otpSecret(typed) ? '' : t('Not a one-time-password secret'))}
        below={
          code && (
            <span className={cx(META, 'flex items-baseline gap-1.5')}>
              <span className="font-medium text-text">{groupDigits(code)}</span>
              <span aria-hidden>·</span>
              <span>{t('refreshes in {{n}}s', { n: time })}</span>
            </span>
          )
        }
      />
    )

  // Reading, the row is worth its place only once a code has arrived. Passing
  // the raw value through when it failed to parse bought nothing but a dead
  // row that copied '' — the backend rejects exactly what `otpSecret` rejects.
  if (!code) return null

  return (
    <FieldRow label={label} actions={<Countdown code={code} time={time} period={period} />}>
      {id => (
        <CopyValue
          id={id}
          value={code}
          label={`${t(label)} · ${t('Copy')}`}
          testid={`entry-value-${name}`}
          // Heavier than a typed value and tracked a touch, so the digits
          // read at a glance; the row's own size otherwise.
          className={cx(
            VALUE_LINE,
            'text-base font-medium tracking-[0.04em] text-text tabular-nums max-md:text-lg'
          )}
        >
          {groupDigits(code)}
        </CopyValue>
      )}
    </FieldRow>
  )
}
