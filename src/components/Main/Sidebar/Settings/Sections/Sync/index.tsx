import { useTranslation } from 'react-i18next'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { ArchiveRestoreGlyph, DiskGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'
import { useBackupStatus } from './Backup/status'
import Summary from './Summary'
import NearbyRow from './NearbyRow'
import CloudRow from './CloudRow'

// Sync & backup, as an overview: where the vault stands, then one row per
// thing there is to do about it, named for the doing: a way of syncing it,
// saving a copy, bringing one back. Each opens a page of its own; nothing is
// switched on or connected from here.
export default function Sync() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const backupStatus = useBackupStatus()

  return (
    <>
      <Summary />

      <SettingsGroup label={t('Sync')}>
        <NearbyRow />
        <CloudRow />
      </SettingsGroup>

      <SettingsGroup label={t('Backup')}>
        <SettingsNavRow
          icon={<DiskGlyph size={16} />}
          label={t('Back up the vault')}
          description={t('A file you keep yourself, sealed with your master password.')}
          status={backupStatus}
          onClick={() => open({ key: 'sync-backup' })}
          testid="settings-sync-open-backup"
        />
        {/* The page Lockboxes offers too: what comes back is a lockbox. Here it
            opens under this section, so Back leads to this overview. */}
        <SettingsNavRow
          icon={<ArchiveRestoreGlyph size={16} />}
          label={t('Restore from a backup')}
          description={t('A backup file and the master password it was sealed with')}
          onClick={() => open({ key: 'restore-backup' })}
          testid="settings-sync-open-restore"
        />
      </SettingsGroup>
    </>
  )
}
