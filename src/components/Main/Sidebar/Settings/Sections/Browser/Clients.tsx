import { useTranslation } from 'react-i18next'
import type { BrowserClient } from '@/api/browser'
import { keyFingerprint } from '@/lib/keyFingerprint'
import { CLIENT_KIND_LABEL, platformLabel } from '@/lib/clientPlatform'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'

// The clients let into the open vault: browser extensions and agents, each
// with what it is (and, for an agent, what it runs on) beside its key.
// Forgetting one ends its access at its next request, connected or not; it
// has to ask, and be let in, again. `busy` is a change already in flight: the
// buttons wait for it.
export default function Clients({
  clients,
  busy,
  onDriven,
  onForget
}: {
  clients: BrowserClient[]
  busy: boolean
  /** Mark an extension as one in a browser an AI agent drives, or not. */
  onDriven: (key: string, driven: boolean) => void
  onForget: (key: string) => void
}) {
  const { t } = useTranslation()
  const drivenLabel = t('Used by an AI agent')

  const describe = (client: BrowserClient) => {
    const platform = platformLabel(client.platform)
    const kind = t(CLIENT_KIND_LABEL[client.kind])
    return platform ? `${kind} · ${platform}` : kind
  }

  return (
    <SettingsGroup label={t('Connected extensions and agents')}>
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
            description={
              <>
                <span data-testid="settings-browser-client-kind">{describe(client)}</span>
                {' · '}
                <span className="font-mono">{keyFingerprint(client.key)}</span>
              </>
            }
            control={
              <div className="flex items-center gap-3">
                {client.kind === 'extension' && (
                  <>
                    <span className="text-sm text-text2">{drivenLabel}</span>
                    <Toggle
                      name="drivenByAgent"
                      checked={!!client.drivenByAgent}
                      disabled={busy}
                      onChange={driven => onDriven(client.key, driven)}
                      aria-label={drivenLabel}
                      testid="settings-browser-client-driven"
                    />
                  </>
                )}
                <Button
                  variant="pale-danger"
                  size="md"
                  disabled={busy}
                  onClick={() => onForget(client.key)}
                  testid="settings-browser-forget"
                >
                  {t('Forget')}
                </Button>
              </div>
            }
          />
        ))
      )}
    </SettingsGroup>
  )
}
