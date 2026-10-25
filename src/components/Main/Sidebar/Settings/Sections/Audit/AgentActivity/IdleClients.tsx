import { useTranslation } from 'react-i18next'
import { browserForgetClient, type BrowserClient } from '@/api/browser'
import { useDates } from '@/hooks/useDates'
import { CLIENT_KIND_LABEL, platformLabel } from '@/lib/clientPlatform'
import Button from '@/components/elements/Button'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'

// The clients let in and not heard from in 90 days — an agent set up for a
// project that ended, a browser profile no longer in use. Each keeps its
// access until forgotten, so the row offers the same Forget as Browser &
// agents, dated by the last time it was seen. One that never proved itself
// again after being let in is dated by that instead; Rust counts it the same
// way. Forgetting re-reads the panel, so the row goes when Rust says so.
export default function IdleClients({
  clients,
  onChange
}: {
  clients: BrowserClient[]
  onChange: () => void
}) {
  const { t } = useTranslation()
  const { shortDate } = useDates()

  const forget = (key: string) =>
    browserForgetClient(key)
      .catch(() => {})
      .finally(onChange)

  const seen = (client: BrowserClient) =>
    client.lastSeenAtMs === undefined
      ? t('never seen, let in {{date}}', { date: shortDate(client.createdAtMs) })
      : t('last seen {{date}}', { date: shortDate(client.lastSeenAtMs) })

  return (
    <SettingsGroup label={t('Agents not seen in 90 days')}>
      {clients.map(client => (
        <SettingsRow
          key={client.key}
          testid="settings-agent-idle-client"
          label={client.name}
          description={[t(CLIENT_KIND_LABEL[client.kind]), platformLabel(client.platform), seen(client)]
            .filter(Boolean)
            .join(' · ')}
          control={
            <Button
              variant="pale-danger"
              size="md"
              onClick={() => void forget(client.key)}
              testid="settings-agent-idle-client-forget"
            >
              {t('Forget')}
            </Button>
          }
        />
      ))}
    </SettingsGroup>
  )
}
