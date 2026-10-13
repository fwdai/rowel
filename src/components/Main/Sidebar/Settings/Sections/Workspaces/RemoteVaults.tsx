import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp, restoreWorkspaceFromAccount } from '@/store'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import DriveRestoreForm from '@/components/elements/DriveRestoreForm'
import { CloudGlyph } from '@/components/Main/icons'

/**
 * The vaults the open workspace's Google account holds that are not on this
 * device — what every device on the account is meant to have. Reported by each
 * sync run (`workspaces:remote`), so a vault made on another device appears
 * here after the next sync, and restored with the account this workspace
 * already has: no sign-in, one password.
 *
 * Nothing is drawn when the list is empty. That is both "every vault is here"
 * and "this workspace does not sync", and neither is worth a row.
 */
export default function RemoteVaults() {
  const { t } = useTranslation()
  const files = useApp(state => state.remoteVaults)
  const [picked, setPicked] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  if (files.length === 0) return null

  // A pick that a later listing no longer carries falls back to the newest.
  const selectedId = files.some(file => file.id === picked) ? picked : files[0].id

  const restore = async (name: string, password: string, fileId: string) => {
    setBusy(true)
    try {
      await restoreWorkspaceFromAccount(name, password, fileId)
    } catch (error) {
      setBusy(false)
      throw error
    }
  }

  return (
    <SettingsGroup label={t('In your Google account')}>
      <SettingsRow
        label={t('Not on this device yet')}
        icon={<CloudGlyph size={16} />}
        description={t(
          'Restore one to add it as a lockbox here, unlocked with its own master password and already syncing. No new sign-in needed.'
        )}
        testid="workspace-remote-row"
      >
        <DriveRestoreForm
          files={files}
          selectedId={selectedId}
          onSelect={setPicked}
          busy={busy}
          onRestore={restore}
          testid="workspace-remote"
          pickerTestid="remote"
        />
      </SettingsRow>
    </SettingsGroup>
  )
}
