import { useTranslation } from 'react-i18next'
import { selectActiveWorkspace, selectWorkspaces, useApp } from '@/store'
import { BACKUP_EXTENSION } from '@/lib/backup'
import { canBackUp } from '@/lib/workspace'
import Chip from '@/components/elements/Chip'
import SettingsRow from '@/components/elements/SettingsRow'
import { CARD } from '@/components/elements/tokens'
import { DiskGlyph } from '@/components/Main/icons'
import BackupRow from './BackupRow'

// Every lockbox on this device that can be saved, the open one first, each a
// file of its own with its own Save… and its own last save. An org lockbox is
// left out: its server keeps it, and its key is the server's.
export default function Saves() {
  const { t } = useTranslation()
  const active = useApp(selectActiveWorkspace)
  const lockboxes = useApp(selectWorkspaces).filter(canBackUp)
  const ordered = [
    ...lockboxes.filter(workspace => workspace.id === active),
    ...lockboxes.filter(workspace => workspace.id !== active)
  ]

  return (
    <div className={CARD} data-testid="settings-backup-saves">
      <SettingsRow
        icon={<DiskGlyph size={16} />}
        label={
          <span className="flex items-center gap-2">
            {t('Save a backup')}
            <Chip>{BACKUP_EXTENSION}</Chip>
          </span>
        }
        description={t('One file per lockbox, sealed with your master password.')}
      />
      {ordered.map(workspace => (
        <BackupRow key={workspace.id} workspace={workspace} />
      ))}
    </div>
  )
}
