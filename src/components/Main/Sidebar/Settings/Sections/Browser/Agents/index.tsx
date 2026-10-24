import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MAX_STANDING_DAYS, MIN_STANDING_DAYS, type AgentsSettings } from '@/api/browser'
import { cx } from '@/utils/cx'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import { inputClass } from '@/components/elements/formStyles'
import { ShieldGlyph } from '@/components/Main/icons'
import Rules from './Rules'
import Grants from './Grants'

// The Agents card: the switch under which every fill in agent mode is a
// grant, the dialog's default length for "allow for N days", and — while the
// switch is on — the rules the user wrote and the grants that are up. Off,
// the lists are not shown: nothing below the switch answers, so there is
// nothing to list.
export default function Agents({
  agents,
  busy,
  onToggle,
  onDays
}: {
  agents: AgentsSettings
  busy: boolean
  onToggle: (enabled: boolean) => void
  onDays: (days: number) => void
}) {
  const { t } = useTranslation()
  // The field's draft while it is being typed in; Rust's value otherwise, so
  // what Settings shows is what the dialog will start at.
  const [draft, setDraft] = useState<number | null>(null)
  const days = draft ?? agents.defaultStandingDays
  const commit = () => {
    setDraft(null)
    if (draft === null || !Number.isFinite(draft)) return
    const next = Math.min(MAX_STANDING_DAYS, Math.max(MIN_STANDING_DAYS, Math.round(draft)))
    if (next !== agents.defaultStandingDays) onDays(next)
  }

  const label = t('Let AI agents use logins through the browser extension')
  const daysLabel = t('Default length of a standing rule')

  return (
    <>
      <SettingsGroup label={t('Agents')}>
        <SettingsRow
          label={label}
          icon={<ShieldGlyph />}
          iconActive={agents.enabled}
          description={t(
            'Every fill is a grant you approve in {{appName}}, or a rule you wrote. Off by default.'
          )}
          control={
            <Toggle
              name="agents"
              checked={agents.enabled}
              disabled={busy}
              onChange={onToggle}
              aria-label={label}
              testid="settings-agents-toggle"
            />
          }
        />
        <SettingsRow
          label={daysLabel}
          description={t('What “Allow for N days” starts at in the grant dialog, from 1 to 90.')}
          control={
            <input
              type="number"
              inputMode="numeric"
              min={MIN_STANDING_DAYS}
              max={MAX_STANDING_DAYS}
              value={days}
              aria-label={daysLabel}
              disabled={busy || !agents.enabled}
              data-testid="settings-agents-days"
              onChange={event => setDraft(event.target.valueAsNumber)}
              onBlur={commit}
              onKeyDown={event => event.key === 'Enter' && commit()}
              className={cx(inputClass, 'w-20')}
            />
          }
        />
      </SettingsGroup>
      {agents.enabled && (
        <>
          <Rules />
          <Grants />
        </>
      )}
    </>
  )
}
