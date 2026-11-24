import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { pickLockboxBackup } from '@/api/pickers'
import type { PickedFile } from '@/api/tools'
import { restoreWorkspaceFromFile, stampSession } from '@/store'
import Button from '@/components/elements/Button'
import FoundFileCard from '@/components/elements/FoundFileCard'
import RestoreForm from '@/components/elements/RestoreForm'
import SettingsRow from '@/components/elements/SettingsRow'
import { DiskGlyph } from '@/components/Main/icons'

// Adding a workspace kept as a backup file: the first run's restore from a
// `.rowel`, made beside the lockboxes already here rather than as the only
// one. It arrives connected to no account.
//
// The backend reads only a file it saw the user choose, so the file comes
// from the picker here, or — `chosen` — from a choice made before this page
// opened: a backup picked or dropped on Import, or opened with the app, whose
// grant is still standing. Either way it is read only when Restore is pressed,
// and a wrong password leaves it chosen for the retry.
//
// Choosing the first file is the row's own control. Choosing another is the
// form's (`aside`), which is what stands it down for every step the form has
// in flight, the move onto the vault password and the open after it included.
export default function RestoreBackup({ chosen }: { chosen: PickedFile | null }) {
  const { t } = useTranslation()
  const [file, setFile] = useState(chosen)
  const [busy, setBusy] = useState(false)

  // A pick that comes back after a lock is the old session's, and is dropped.
  const pick = () => {
    const current = stampSession()
    return pickLockboxBackup()
      .then(picked => {
        if (picked && current()) setFile(picked)
      })
      .catch(() => {})
  }

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
        !file && (
          <Button
            variant="pale"
            size="md"
            testid="workspace-backup-pick"
            onClick={() => void pick()}
          >
            {t('Choose file…')}
          </Button>
        )
      }
    >
      {file && (
        <RestoreForm
          target={file.path}
          busy={busy}
          onRestore={restore}
          wrongPassword={t('Invalid password for backup')}
          aside={
            <Button variant="pale" testid="workspace-backup-pick" onClick={() => void pick()}>
              {t('Pick another file')}
            </Button>
          }
          testid="workspace-backup"
        >
          <FoundFileCard
            where="disk"
            name={file.name}
            encrypted
            testid="workspace-backup-file"
          />
        </RestoreForm>
      )}
    </SettingsRow>
  )
}
