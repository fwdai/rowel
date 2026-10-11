import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { copy } from '@/services/copy'
import { cx } from '@/utils/cx'
import { useOtp } from '@/hooks/useOtp'
import Panel from '../../Panel'
import { verbatimInput } from '../../inputProps'
import { LABEL, VALUE_LINE } from '../../tokens'
import { useField } from '../context'
import FieldRow from '../Row'
import Countdown from './Countdown'
import Dial from './Dial'
import { groupDigits, otpSecret, otpStored } from './secret'

/**
 * The one-time code, in both modes.
 *
 * Reading, it is a row like the credentials around it — the current code as
 * the value, with how long it has left as a ring in the rail — the way
 * Passwords and 1Password list a verification code under the password rather
 * than in a widget of its own. The value is the copy control, as every row's
 * is. Editing, it is a panel: the secret's own input with the code as a live
 * preview on the dial — the only proof that what was pasted actually works.
 */
export default function OtpField({
  name = 'otp',
  label = 'One-time code',
  autoFocus
}: {
  name?: string
  label?: TKey
  /** The column was just opened to add a code: put the caret in the box. */
  autoFocus?: boolean
}) {
  const { t } = useTranslation()
  const { value, set, editing } = useField(name)
  const parsed = otpSecret(value)
  // `parsed` answers "is this a secret at all"; `stored` is the thing worth
  // keeping — the same seed, plus the parameters when the link carries any the
  // generator would otherwise have to guess at. The backend reads both forms.
  const stored = otpStored(value)
  const { code, time, period } = useOtp(stored)

  // Reading, the row is worth its place only once a code has arrived. Passing
  // the raw value through when it failed to parse bought nothing but a dead
  // row that copied '' — the backend rejects exactly what `otpSecret` rejects.
  if (!editing && !code) return null

  if (!editing)
    return (
      <FieldRow label={label} actions={<Countdown code={code} time={time} period={period} />}>
        {id => (
          <button
            id={id}
            type="button"
            aria-label={`${t(label)} · ${t('Copy')}`}
            onClick={() => copy(code)}
            data-testid={`entry-value-${name}`}
            // Heavier than a typed value and tracked a touch, so the digits
            // read at a glance as the dial's did; the row's own size otherwise.
            className={cx(
              VALUE_LINE,
              'cursor-pointer text-left text-base font-medium tracking-[0.04em] text-text tabular-nums max-md:text-lg'
            )}
          >
            {groupDigits(code)}
          </button>
        )}
      </FieldRow>
    )

  return (
    <Panel className="flex flex-col items-center p-3.5">
      <div className={`self-stretch ${LABEL}`}>{t(label)}</div>

      <input
        name={name}
        // The panel's heading is not a label element, so the input names
        // itself rather than borrowing the row geometry it does not use.
        aria-label={t(label)}
        value={value}
        // Short enough to fit the column: the long form used to truncate mid-word.
        placeholder={t('Paste secret or link')}
        autoFocus={autoFocus}
        {...verbatimInput}
        onChange={event => set(event.target.value)}
        // A pasted otpauth:// link collapses to the secret it carries —
        // unless it also carries a digit count, period or algorithm of its
        // own, which the link is the only place to keep.
        onBlur={() => set(stored || value.trim())}
        className={`mt-2.5 h-6 w-full self-stretch truncate border-b bg-transparent text-center text-base text-text outline-none transition-colors placeholder:text-text2 ${
          value && !parsed ? 'border-bad' : 'border-line2 focus:border-accent-line'
        }`}
      />

      {/* The preview is not copyable: while typing, the dial is proof the
          secret works, not a thing to take the code from. */}
      {parsed && <Dial code={code} time={time} period={period} />}

      {value !== '' && !parsed && (
        <div className="mt-3 text-base text-bad">{t('Not a one-time-password secret')}</div>
      )}
    </Panel>
  )
}
