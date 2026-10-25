import { useTranslation } from 'react-i18next'
import { agentRevokeRule, type AgentRule } from '@/api/browser'
import { useDates } from '@/hooks/useDates'
import Button from '@/components/elements/Button'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { modeWords } from '../../Browser/Agents/mode'

// The standing rules written over 30 days ago and still in force: the ones
// most likely to have outlived the task they were written for. The row reads
// as it does in Browser & agents, dated by when it was written rather than
// when it ends, since its age is the point. Removing one re-reads the panel,
// so the row goes when Rust says it has gone.
export default function StaleRules({
  rules,
  onChange
}: {
  rules: AgentRule[]
  onChange: () => void
}) {
  const { t } = useTranslation()
  const { shortDate } = useDates()

  const remove = (id: string) =>
    agentRevokeRule(id)
      .catch(() => {})
      .finally(onChange)

  return (
    <SettingsGroup label={t('Standing rules older than 30 days')}>
      {rules.map(rule => (
        <SettingsRow
          key={rule.id}
          testid="settings-agent-stale-rule"
          label={rule.itemTitle ?? t('Untitled')}
          description={[
            rule.target ?? rule.origin,
            modeWords(t, rule.mode),
            rule.clientName,
            t('written {{date}}', { date: shortDate(rule.createdAtMs) })
          ]
            .filter(Boolean)
            .join(' · ')}
          control={
            <Button
              variant="pale-danger"
              size="md"
              onClick={() => void remove(rule.id)}
              testid="settings-agent-stale-rule-remove"
            >
              {t('Remove')}
            </Button>
          }
        />
      ))}
    </SettingsGroup>
  )
}
