import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { agentEndGrant, agentGrants, type AgentGrant } from '@/api/browser'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import { getLocale } from '@/i18n'
import { useUi } from '@/store'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { modeWords } from './mode'

const MINUTE = 60_000
const HOUR = 60 * MINUTE

// "in 12 minutes", "in 2 hours": the time a grant has left, spelled by Intl
// so no locale carries a key per unit. Never less than a minute away — a
// grant that close is as good as live until the next re-read drops it.
const timeLeft = (atMs: number, now = Date.now()) => {
  const left = Math.max(atMs - now, MINUTE)
  const spell = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'always' })
  return left < HOUR
    ? spell.format(Math.ceil(left / MINUTE), 'minute')
    : spell.format(Math.ceil(left / HOUR), 'hour')
}

// The grants Rust holds: the active ones, which can be ended here, and the
// ones that ended within the hour, so what just happened is still in view.
// Re-read on `agents:changed` — a grant issued, expired or ended elsewhere.
export default function Grants() {
  const { t } = useTranslation()
  const seq = useUi(state => state.agentsSeq)
  const [grants, setGrants] = useState<AgentGrant[] | null>(null)
  const [reread, setReread] = useState(0)
  const begin = useLatestRequest()

  useEffect(() => {
    const current = begin()
    agentGrants()
      .then(list => current() && setGrants(list))
      .catch(() => current() && setGrants([]))
  }, [begin, seq, reread])

  const end = (id: string) =>
    agentEndGrant(id)
      .catch(() => {})
      .finally(() => setReread(n => n + 1))

  const standing = (grant: AgentGrant) =>
    grant.state === 'active'
      ? t('Expires {{when}}', { when: timeLeft(grant.expiresAtMs) })
      : t('Ended')

  return (
    <SettingsGroup label={t('Active grants')}>
      {grants && grants.length === 0 ? (
        <Note testid="settings-agents-grants-empty" className="px-4 py-3.5">
          {t('No agent is using a login right now.')}
        </Note>
      ) : (
        grants?.map(grant => (
          <SettingsRow
            key={grant.id}
            testid="settings-agents-grant"
            label={grant.itemTitle ?? t('Untitled')}
            description={[grant.clientName, grant.target, modeWords(t, grant.mode), standing(grant)]
              .filter(Boolean)
              .join(' · ')}
            control={
              grant.state === 'active' ? (
                <Button
                  variant="pale-danger"
                  size="md"
                  onClick={() => void end(grant.id)}
                  testid="settings-agents-grant-end"
                >
                  {t('End')}
                </Button>
              ) : undefined
            }
          />
        ))
      )}
    </SettingsGroup>
  )
}
