import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { MAX_STANDING_DAYS, MIN_STANDING_DAYS, type GrantMode } from '@/api/browser'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import { inputClass } from '@/components/elements/formStyles'
import { clampDays } from './days'

/**
 * The answers: this once; for this run, when the request named one (`runHours`
 * is then the most it lasts); for N days (which writes a rule for this
 * client, this item and this place, in this mode); or no. The run is the
 * client's own word, so its note promises no more than what the client marks
 * as the same run, for at most the hours. `days` is editable
 * in place; the field commits a clamped value when it loses focus, and the
 * standing button clamps again on the way out, so Rust is never sent a length
 * it would refuse. The note under the days says what the rule would let
 * through, in the mode's own terms: a fill on a site, a header to an API, a
 * value to one command line.
 */
export default function Choices({
  mode,
  several,
  days,
  onDays,
  disabled,
  runHours,
  onOnce,
  onRun,
  onStanding,
  onDeny
}: {
  mode: GrantMode
  /** A secret ask naming more than one item: the rule would name them all. */
  several: boolean
  days: number
  onDays: (days: number) => void
  /** A yes is not available yet — a step-up ask with no password typed. */
  disabled: boolean
  /** Set when the request named a run: the most a yes for it lasts. */
  runHours?: number
  onOnce: () => void
  onRun: () => void
  onStanding: () => void
  onDeny: () => void
}) {
  const { t } = useTranslation()
  const daysId = useId()
  const standingLabel =
    days === 1 ? t('Allow for 1 day') : t('Allow for {{days}} days', { days })
  const note =
    mode === 'header'
      ? several
        ? t(
            'Allowing for days writes a rule: this client sends these items in requests to that place without asking until then. Remove it any time in Settings › Browser & agents.'
          )
        : t(
            'Allowing for days writes a rule: this client sends this item in requests to that place without asking until then. Remove it any time in Settings › Browser & agents.'
          )
      : mode === 'env'
        ? several
          ? t(
              'Allowing for days writes a rule: this client gives these items to exactly that command line without asking until then. Remove it any time in Settings › Browser & agents.'
            )
          : t(
              'Allowing for days writes a rule: this client gives this item to exactly that command line without asking until then. Remove it any time in Settings › Browser & agents.'
            )
        : t(
            'Allowing for days writes a rule: this client fills this login on this site without asking until then. Remove it any time in Settings › Browser & agents.'
          )

  return (
    <div className="mt-5 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="md" onClick={onOnce} disabled={disabled} testid="agent-grant-once">
          {t('Allow once')}
        </Button>
        {runHours !== undefined && (
          <Button
            variant="ghost"
            size="md"
            onClick={onRun}
            disabled={disabled}
            testid="agent-grant-run"
          >
            {t('Allow for this run')}
          </Button>
        )}
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
      {runHours !== undefined && (
        <p className="text-sm text-text2" data-testid="agent-grant-run-note">
          {t(
            'Allowing for this run lets through what this client marks as the same run, without asking, for at most {{hours}} hours. The client decides what a run is, so the hours are the limit.',
            { hours: runHours }
          )}
        </p>
      )}
      <p className="text-sm text-text2" data-testid="agent-grant-standing-note">
        {note}
      </p>
    </div>
  )
}
