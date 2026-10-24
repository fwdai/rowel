import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { agentRevokeRule, agentRules, type AgentRule } from '@/api/browser'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import { useDates } from '@/hooks/useDates'
import { useUi } from '@/store'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'

// The standing rules the user wrote by answering "allow for N days": which
// login, on which site, for which client, until when. Read on the way in and
// again whenever Rust says a rule came or went (`agents:changed`); removing
// one re-reads too, so the row goes when Rust says it has gone.
export default function Rules() {
  const { t } = useTranslation()
  const { shortDate } = useDates()
  const seq = useUi(state => state.agentsSeq)
  const [rules, setRules] = useState<AgentRule[] | null>(null)
  const [reread, setReread] = useState(0)
  const begin = useLatestRequest()

  useEffect(() => {
    const current = begin()
    agentRules()
      .then(list => current() && setRules(list))
      .catch(() => current() && setRules([]))
  }, [begin, seq, reread])

  const remove = (id: string) =>
    agentRevokeRule(id)
      .catch(() => {})
      .finally(() => setReread(n => n + 1))

  const until = (rule: AgentRule) =>
    rule.expiresAtMs === null
      ? t('until removed')
      : t('until {{date}}', { date: shortDate(rule.expiresAtMs) })

  return (
    <SettingsGroup label={t('Rules')}>
      {rules && rules.length === 0 ? (
        <Note testid="settings-agents-rules-empty" className="px-4 py-3.5">
          {t('No standing rules yet. Answer “Allow for N days” in the grant dialog to write one.')}
        </Note>
      ) : (
        rules?.map(rule => (
          <SettingsRow
            key={rule.id}
            testid="settings-agents-rule"
            label={rule.itemTitle ?? t('Untitled')}
            description={[rule.origin, rule.clientName, until(rule)].filter(Boolean).join(' · ')}
            control={
              <Button
                variant="pale-danger"
                size="md"
                onClick={() => void remove(rule.id)}
                testid="settings-agents-rule-remove"
              >
                {t('Remove')}
              </Button>
            }
          />
        ))
      )}
    </SettingsGroup>
  )
}
