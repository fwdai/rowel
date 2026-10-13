import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import {
  useApp,
  connectWorkspaceDrive,
  forgetDrive,
  restoreWorkspaceFromDrive,
  setupDriveSelect,
  switchWorkspaceDriveAccount
} from '@/store'
import SettingsRow from '@/components/elements/SettingsRow'
import Button from '@/components/elements/Button'
import DriveRestoreForm from '@/components/elements/DriveRestoreForm'
import Note from '@/components/elements/Note'
import { CloudGlyph } from '@/components/Main/icons'

// Adding a workspace that already exists somewhere else.
//
// A second device can restore exactly one vault, during onboarding; every other
// vault in the same Google account was unreachable from it. This is the way in
// to those: the account is connected while a vault is open here, its tokens are
// held in memory, and the vault the user picks arrives as a workspace of its
// own — already syncing, since the account is sealed under its key on the way.
//
// Everything on screen follows the probe in the store, exactly as the first
// run's Drive screen does, and for the same reason: the answer arrives as an
// event, so a result that lands while the user is reading simply redraws this.
export default function RestoreFromDrive() {
  const { t } = useTranslation()
  const drive = useApp(state => state.setupDrive)

  // Leaving is backing out. A sign-in the user walked away from must never be
  // adopted by a later create or restore, so the pending tokens go with the
  // screen that asked for them — whether it goes because the user pressed
  // Cancel, moved to another section, or closed Settings. Harmless after a
  // restore that worked: the backend has already taken them. And harmless
  // during one: the backend holds the account for the restore's length and
  // refuses to drop it, so leaving mid-restore lets what the user asked for
  // finish — exactly as leaving mid-create does.
  useEffect(() => forgetDrive, [])

  const restoring = drive.status === 'restoring'

  return (
    <SettingsRow
      label={t('Restore from Google Drive')}
      icon={<CloudGlyph size={16} />}
      description={t(
        'Sign in and pick one of the lockboxes this Google account already holds. It arrives as a separate lockbox, unlocked with its own master password and syncing to the same pack.'
      )}
      testid="workspace-restore-row"
      control={
        drive.status === 'idle' ? (
          <Button size="md" testid="workspace-restore-connect" onClick={connectWorkspaceDrive}>
            {t('Connect')}
          </Button>
        ) : restoring ? null : (
          // Withdrawn while the restore runs, with the form's own Switch
          // account: the account is spoken for, and a Cancel that the backend
          // would refuse is a lie.
          <Button
            variant="pale"
            size="md"
            testid="workspace-restore-cancel"
            onClick={forgetDrive}
          >
            {t('Cancel')}
          </Button>
        )
      }
    >
      {drive.status === 'pending' && (
        <Note testid="workspace-restore-waiting">{t('Waiting for Google…')}</Note>
      )}
      {drive.status === 'empty' && (
        <Note testid="workspace-restore-empty">
          {t('This Google account has nothing to restore')}
        </Note>
      )}
      {drive.status === 'error' && (
        <Note tone="bad" testid="workspace-restore-error">
          {drive.error || t('Something went wrong')}
        </Note>
      )}
      {(drive.status === 'found' || restoring) && (
        <DriveRestoreForm
          files={drive.files}
          selectedId={drive.selectedId}
          onSelect={setupDriveSelect}
          busy={restoring}
          onRestore={restoreWorkspaceFromDrive}
          onSwitchAccount={switchWorkspaceDriveAccount}
        />
      )}
    </SettingsRow>
  )
}
