import { useTranslation } from 'react-i18next'
import type { BrowserClient } from '@/api/browser'
import { keyFingerprint } from '@/lib/keyFingerprint'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'

// The extensions let into the open vault. Forgetting one ends its access at
// its next request, connected or not; it has to ask, and be let in, again.
// `busy` is a change already in flight: the buttons wait for it.
export default function Clients({
  clients,
  busy,
  onForget
}: {
  clients: BrowserClient[]
  busy: boolean
  onForget: (key: string) => void
}) {
  const { t } = useTranslation()

  return (
    <SettingsGroup label={t('Connected extensions')}>
      {clients.length === 0 ? (
        <Note testid="settings-browser-clients-empty" className="px-4 py-3.5">
          {t('No extension has connected to this vault yet')}
        </Note>
      ) : (
        clients.map(client => (
          <SettingsRow
            key={client.key}
            testid="settings-browser-client"
            label={client.name}
            description={<span className="font-mono">{keyFingerprint(client.key)}</span>}
            control={
              <Button
                variant="pale-danger"
                size="md"
                disabled={busy}
                onClick={() => onForget(client.key)}
                testid="settings-browser-forget"
              >
                {t('Forget')}
              </Button>
            }
          />
        ))
      )}
    </SettingsGroup>
  )
}
