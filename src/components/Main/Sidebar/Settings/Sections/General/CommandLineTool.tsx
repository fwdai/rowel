import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cliInstall, cliStatus, type CliStatus } from '@/api/browser'
import { describeError } from '@/api/errors'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { SshGlyph } from '@/components/Main/icons'

// Settings › General › Command line tool: the `rowel` command coding agents
// reach the vault through (its MCP server), shipped beside the packaged app.
// Three states, all Rust's to say: linked into the PATH already (the path is
// shown), bundled but not yet linked (one button does it), or not bundled at
// all — a dev build, until `cargo build -p rowel-cli` puts one beside the
// binary. Desktop only; the caller leaves it out on a phone.
export default function CommandLineTool() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<CliStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // An install's answer and a late status read must not cross: the newest
  // request is the status as it stands.
  const begin = useLatestRequest()
  const load = useCallback(
    (request: () => Promise<CliStatus>) => {
      const current = begin()
      return request()
        .then(next => {
          if (!current()) return
          setStatus(next)
          setError(null)
        })
        .catch(reason => current() && setError(describeError(reason)))
    },
    [begin]
  )
  useEffect(() => {
    void load(cliStatus)
  }, [load])

  const install = () => {
    setBusy(true)
    void load(cliInstall).finally(() => setBusy(false))
  }

  const label = t('Command line tool')
  const description = !status
    ? null
    : status.installed
      ? t('Installed as {{path}}. Coding agents connect through its MCP server.', {
          path: status.installed
        })
      : status.bundled
        ? t(
            'Puts the rowel command on your PATH, so coding agents can connect through its MCP server.'
          )
        : t('The command line tool ships with the packaged app; this build has none beside it.')
  const state = !status
    ? undefined
    : status.installed
      ? 'installed'
      : status.bundled
        ? 'bundled'
        : 'missing'

  return (
    <SettingsGroup label={label}>
      <SettingsRow
        label={t('The rowel command')}
        icon={<SshGlyph />}
        iconActive={!!status?.installed}
        description={description && <span data-testid={`settings-cli-${state}`}>{description}</span>}
        control={
          status && !status.installed && status.bundled ? (
            <Button size="md" disabled={busy} onClick={install} testid="settings-cli-install">
              {t('Install')}
            </Button>
          ) : undefined
        }
        testid="settings-cli-row"
      >
        {error && (
          <Note tone="bad" testid="settings-cli-error">
            {error}
          </Note>
        )}
      </SettingsRow>
    </SettingsGroup>
  )
}
