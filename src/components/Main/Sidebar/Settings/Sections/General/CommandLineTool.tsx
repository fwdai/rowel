import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cliInstall, cliStatus, cliUninstall, type CliStatus } from '@/api/browser'
import { describeError } from '@/api/errors'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import Button from '@/components/elements/Button'
import CopyButton from '@/components/elements/CopyButton'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { SshGlyph } from '@/components/Main/icons'

// Settings › General › Command line tool: the `rowel` command coding agents
// reach the vault through (its MCP server), shipped beside the packaged app.
// Three states, all Rust's to say: on the PATH already (Remove takes it off),
// bundled but not yet there (Install puts it there), or not bundled at all —
// a dev build, until `cargo build -p rowel-cli` puts one beside the binary.
// Whenever there is a tool its full path is shown, with a copy button: an
// agent started from the Dock or the Start menu does not read the shell's
// PATH, so its MCP config names the tool by that path. Desktop only; the
// caller leaves it out on a phone.
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

  const act = (request: () => Promise<CliStatus>) => {
    setBusy(true)
    void load(request).finally(() => setBusy(false))
  }

  const label = t('Command line tool')
  // A directory the PATH of a new terminal does not list — `~/.local/bin` on
  // a Mac whose shell profile does not add it — has to be added by hand.
  const needsPathHint = !!status?.installed && !status.onPath
  const path = status?.installed ?? status?.bundled
  const description = !status
    ? null
    : status.installed
      ? needsPathHint
        ? t(
            'Installed as {{path}}. Coding agents connect through its MCP server. If your terminal does not find rowel, add {{dir}} to your PATH.',
            { path: status.installed, dir: status.installDir }
          )
        : t('Installed as {{path}}. Coding agents connect through its MCP server.', {
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
          status?.installed ? (
            <Button
              size="md"
              disabled={busy}
              onClick={() => act(cliUninstall)}
              testid="settings-cli-uninstall"
            >
              {t('Remove')}
            </Button>
          ) : status?.bundled ? (
            <Button
              size="md"
              disabled={busy}
              onClick={() => act(cliInstall)}
              testid="settings-cli-install"
            >
              {t('Install')}
            </Button>
          ) : undefined
        }
        testid="settings-cli-row"
      >
        {path && (
          <div className="text-sm text-text2">
            {t("Use this full path in an agent's MCP config:")}
            <div className="mt-1 flex items-center gap-2">
              <code
                className="min-w-0 flex-1 font-mono break-all text-text select-text"
                data-testid="settings-cli-path"
              >
                {path}
              </code>
              <CopyButton value={path} title={t('Copy')} testid="settings-cli-copy" stay />
            </div>
          </div>
        )}
        {error && (
          <Note tone="bad" testid="settings-cli-error">
            {error}
          </Note>
        )}
      </SettingsRow>
    </SettingsGroup>
  )
}
