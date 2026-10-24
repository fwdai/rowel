import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { MAX_STANDING_DAYS, MIN_STANDING_DAYS } from '@/api/browser'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import { inputClass } from '@/components/elements/formStyles'
import { clampDays } from './days'

/**
 * The three answers: this once, for N days (which writes a rule for this
 * client, this login and this site), or no. `days` is editable in place; the
 * field commits a clamped value when it loses focus, and the standing button
 * clamps again on the way out, so Rust is never sent a length it would refuse.
 */
export default function Choices({
  days,
  onDays,
  disabled,
  onOnce,
  onStanding,
  onDeny
}: {
  days: number
  onDays: (days: number) => void
  /** A yes is not available yet — a step-up ask with no password typed. */
  disabled: boolean
  onOnce: () => void
  onStanding: () => void
  onDeny: () => void
}) {
  const { t } = useTranslation()
  const daysId = useId()
  const standingLabel =
    days === 1 ? t('Allow for 1 day') : t('Allow for {{days}} days', { days })

  return (
    <div className="mt-5 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="md" onClick={onOnce} disabled={disabled} testid="agent-grant-once">
          {t('Allow once')}
        </Button>
        <Button
          variant="ghost"
          size="md"
          onClick={onStanding}
          disabled={disabled}
          testid="agent-grant-standing"
        >
          {standingLabel}
        </Button>
        <Button variant="pale" size="md" onClick={onDeny} testid="agent-grant-deny">
          {t('Deny')}
        </Button>
      </div>
      <label htmlFor={daysId} className="flex items-center gap-2 text-sm text-text2">
        {t('Days')}
        <input
          id={daysId}
          type="number"
          inputMode="numeric"
          min={MIN_STANDING_DAYS}
          max={MAX_STANDING_DAYS}
          value={days}
          data-testid="agent-grant-days"
          onChange={event => onDays(event.target.valueAsNumber)}
          onBlur={event => onDays(clampDays(event.target.valueAsNumber, days))}
          className={cx(inputClass, 'w-20')}
        />
      </label>
      <p className="text-sm text-text2">
        {t(
          'Allowing for days writes a rule: this client fills this login on this site without asking until then. Remove it any time in Settings › Browser & agents.'
        )}
      </p>
    </div>
  )
}
