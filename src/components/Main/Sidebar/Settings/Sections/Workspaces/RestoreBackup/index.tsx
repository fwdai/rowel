import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { pickLockboxBackup } from '@/api/pickers'
import { restoreWorkspaceFromFile } from '@/store'
import Button from '@/components/elements/Button'
import FoundFileCard from '@/components/elements/FoundFileCard'
import RestoreForm from '@/components/elements/RestoreForm'
import SettingsRow from '@/components/elements/SettingsRow'
import { DiskGlyph } from '@/components/Main/icons'
import { fileNameOf } from '@/components/Start/shared/describe'

// Adding a workspace kept as a backup file: the first run's restore from a
// `.rowel`, made beside the lockboxes already here rather than as the only
// one. It arrives connected to no account.
//
// The backend reads only a file it saw the user choose, so the file comes
// from the picker here, or — `chosen` — from a choice made before this page
// opened: a backup picked or dropped on Import, or opened with the app, whose
// grant is still standing. Either way it is read only when Restore is pressed,
// and a wrong password leaves it chosen for the retry.
export default function RestoreBackup({ chosen }: { chosen: string | null }) {
  const { t } = useTranslation()
  const [path, setPath] = useState(chosen)
  const [busy, setBusy] = useState(false)

  const pick = () =>
    pickLockboxBackup()
      .then(picked => {
        if (picked) setPath(picked)
      })
      .catch(() => {})

  // Settings is held by the store for the restore's length; this only stands
  // the controls down, and puts them back for a retry.
  const restore = async (name: string, password: string, target: string) => {
    setBusy(true)
    try {
      await restoreWorkspaceFromFile(name, password, target)
    } catch (error) {
      setBusy(false)
      throw error
    }
  }

  return (
    <SettingsRow
      icon={<DiskGlyph size={16} />}
      label={t('Rowel backup')}
      description={t('Use the master password you had when this backup was made.')}
      testid="workspace-backup-row"
      control={
        busy ? null : (
          <Button
            variant="pale"
            size="md"
            testid="workspace-backup-pick"
            onClick={() => void pick()}
          >
            {path ? t('Pick another file') : t('Choose file…')}
          </Button>
        )
      }
    >
      {path && (
        <RestoreForm
          target={path}
          busy={busy}
          onRestore={restore}
          wrongPassword={t('Invalid password for backup')}
          testid="workspace-backup"
        >
          <FoundFileCard
            where="disk"
            name={fileNameOf(path)}
            encrypted
            testid="workspace-backup-file"
          />
        </RestoreForm>
      )}
    </SettingsRow>
  )
}
